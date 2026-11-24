pragma Singleton
import QtQuick
import Quickshell
import Quickshell.Io

QtObject {
    id: backend
    property var workspace: ({
            sessions: {},
            layout: {
                pane_ids: [],
                selected: "",
                detail: "quiet",
                inspector: "context"
            }
        })
    property var runtime: ({
            mode: "offline",
            connected: false,
            active: [],
            busy: [],
            permissions: {},
            providers: []
        })
    property bool activated: false
    function ensureStarted() {
        if (!closing) activated = true;
    }
    property bool available: false
    property string notice: "Opening local workspace…"
    property bool noticeError: false
    property var callbacks: ({})
    property int sequence: 0
    property var histories: ({})
    property var historyNotes: ({})
    property var drafts: ({})
    property var dirty: ({})
    property var saving: ({})
    property var inFlight: ({})
    property string nextCursor: ""
    property bool restored: false
    property var automationResults: ({})
    property int automationSequence: 0
    property bool closing: false
    property var sessions: Object.values(workspace.sessions).sort((a, b) => b.updated_at - a.updated_at)
    property string selected: workspace.layout.selected || ""
    property var panes: workspace.layout.pane_ids || []
    property bool fullDetail: workspace.layout.detail === "full"
    signal shutdownReady
    signal updateReceived(string sessionId, var update)
    signal sessionActivated(string oldId, string newId)

    property Process bridge: Process {
        command: [Quickshell.env("OVRIN_HARNESS_BIN"), Quickshell.env("OVRIN_HARNESS_CONFIG"), Quickshell.env("OVRIN_HARNESS_DATA"), Quickshell.env("OVRIN_HARNESS_MODE") || "offline"]
        stdinEnabled: true
        running: backend.activated || Quickshell.env("OVRIN_HARNESS_AUTOMATION") === "1"
        onStarted: {
            backend.available = true;
            backend.send("state", {}, r => backend.install(r));
        }
        onRunningChanged: if (!running) {
            backend.available = false;
            const waiting = backend.callbacks;
            backend.callbacks = ({});
            Object.keys(waiting).forEach(id => waiting[id](null, false));
            backend.inFlight = ({});
            backend.saving = ({});
            backend.notice = "Workspace process stopped. Unsaved text remains in this window; saved drafts remain on disk.";
            backend.noticeError = true;
        }
        stdout: SplitParser {
            onRead: data => {
                try {
                    const m = JSON.parse(data);
                    if (m.event === "state")
                        backend.install(m.data);
                    else if (m.event === "notice")
                        backend.message(m.text, m.error);
                    else if (m.event === "session_update")
                        backend.append(m.session_id, {
                            kind: "update",
                            data: m.data
                        });
                    else if (m.event === "turn_finished")
                        backend.loadHistory(m.session_id);
                    else if (m.id !== undefined) {
                        const cb = backend.callbacks[m.id];
                        delete backend.callbacks[m.id];
                        if (!m.ok)
                            backend.message(m.error, true);
                        if (cb)
                            cb(m.result, m.ok);
                    }
                } catch (e) {
                    backend.message("Could not decode workspace response: " + e, true);
                }
            }
        }
        stderr: SplitParser {
            onRead: data => backend.message(data, true)
        }
    }
    property Timer saveTimer: Timer {
        interval: 350
        onTriggered: backend.flushDrafts()
    }
    // Explicit, local-only acceptance interface. Disabled on normal launches.
    property IpcHandler acceptance: IpcHandler {
        target: "workbench"
        enabled: Quickshell.env("OVRIN_HARNESS_AUTOMATION") === "1"
        function snapshot(): string {
            return JSON.stringify({
                workspace: backend.workspace,
                runtime: backend.runtime,
                notice: backend.notice,
                available: backend.available,
                results: backend.automationResults
            });
        }
        function request(command: string, argumentsJson: string): int {
            const id = ++backend.automationSequence;
            const args = JSON.parse(argumentsJson);
            backend.send(command, args, (result, ok) => {
                let outcomes = Object.assign({}, backend.automationResults);
                outcomes[id] = {
                    ok: ok,
                    result: result,
                    notice: backend.notice
                };
                Object.keys(outcomes).map(Number).sort((a, b) => b - a).slice(32).forEach(k => delete outcomes[k]);
                backend.automationResults = outcomes;
            });
            return id;
        }
        function editDraft(sessionId: string, text: string) {
            backend.editDraft(sessionId, text);
        }
        function select(sessionId: string, split: bool) {
            backend.select(sessionId, split);
        }
        function openSession(sessionId: string) {
            backend.openSession(sessionId);
        }
        function submit(sessionId: string) {
            backend.submit(sessionId);
        }
        function close() {
            backend.shutdown();
        }
    }
    property Timer closeTimer: Timer {
        interval: 50
        repeat: true
        onTriggered: {
            if (Object.keys(backend.saving).length)
                return;
            stop();
            if (Object.keys(backend.dirty).length) {
                backend.closing = false;
                backend.message("Close cancelled: a draft could not be saved. Resolve the error before closing.", true);
                return;
            }
            backend.send("disconnect", {}, (r, ok) => {
                if (ok)
                    backend.shutdownReady();
                else {
                    backend.closing = false;
                    backend.message("Could not shut down cleanly; window retained.", true);
                }
            });
        }
    }
    function shutdown() {
        if (closing)
            return;
        closing = true;
        if (!available) {
            if (Object.keys(dirty).length) {
                closing = false;
                message("Unsaved console text remains. Reconnect before closing.", true);
            } else
                shutdownReady();
            return;
        }
        flushDrafts();
        closeTimer.start();
    }

    function message(text, error) {
        notice = text;
        noticeError = !!error;
    }
    function install(snapshot) {
        if (!snapshot || !snapshot.workspace)
            return;
        const wasConnected = runtime.connected;
        workspace = snapshot.workspace;
        runtime = snapshot.runtime;
        if (!wasConnected && runtime.connected)
            message("Zeno connected · " + runtime.mode, false);
        if (!restored) {
            restored = true;
            (workspace.layout.pane_ids || []).forEach(id => loadHistory(id));
        }
        if (notice === "Opening local workspace…")
            message("Local workspace ready. " + (runtime.mode === "offline" ? "Inference is disabled." : "Connect the engine when ready."), false);
    }
    function send(command, args, callback) {
        if (!available) {
            message("Workspace process is unavailable", true);
            if (callback)
                callback(null, false);
            return;
        }
        const id = ++sequence;
        if (callback)
            callbacks[id] = callback;
        bridge.write(JSON.stringify({
            id: id,
            command: command,
            args: args
        }) + "\n");
    }
    function getDraft(id) {
        return drafts[id] !== undefined ? drafts[id] : (workspace.sessions[id] || {}).draft || "";
    }
    function editDraft(id, text) {
        let d = Object.assign({}, drafts);
        d[id] = text;
        drafts = d;
        dirty[id] = true;
        saveTimer.restart();
    }
    function saveDraft(id, after) {
        const text = getDraft(id);
        saving[id] = true;
        send("draft.save", {
            session_id: id,
            text: text
        }, (r, ok) => {
            delete saving[id];
            if (ok && getDraft(id) === text)
                delete dirty[id];
            if (after)
                after(ok);
        });
    }
    function flushDrafts() {
        Object.keys(dirty).forEach(id => {
            if (!saving[id])
                saveDraft(id);
        });
    }
    function isBusy(id) {
        return (runtime.busy || []).indexOf(id) >= 0 || !!inFlight[id];
    }
    function isActive(id) {
        return (runtime.active || []).indexOf(id) >= 0;
    }
    function select(id, split) {
        let ids = panes.slice();
        if (split && ids.indexOf(id) < 0) {
            if (ids.length >= 3)
                ids[ids.length - 1] = id;
            else
                ids.push(id);
        } else if (!split)
            ids = [id];
        saveLayout(ids, id, fullDetail);
        loadHistory(id);
    }
    function saveLayout(ids, selectedId, full) {
        send("layout.save", {
            pane_ids: ids,
            selected: selectedId,
            detail: full ? "full" : "quiet",
            inspector: workspace.layout.inspector || "context"
        });
    }
    function closePane(id) {
        const ids = panes.filter(s => s !== id);
        saveLayout(ids, ids[0] || "", fullDetail);
    }
    function loadHistory(id) {
        if (!id)
            return;
        send("history", {
            session_id: id
        }, (r, ok) => {
            if (!ok)
                return;
            const h = Object.assign({}, histories);
            h[id] = r.events;
            histories = h;
            const n = Object.assign({}, historyNotes);
            n[id] = (r.partial ? "Showing a bounded recent observation window. Full observations remain on disk. " : "") + (r.invalid_records ? r.invalid_records + " incomplete records retained on disk." : "");
            historyNotes = n;
        });
    }
    function append(id, event) {
        const h = Object.assign({}, histories);
        h[id] = (h[id] || []).concat([event]).slice(-500);
        histories = h;
        updateReceived(id, event.data);
    }
    function openSession(id) {
        saveDraft(id, ok => {
            if (!ok)
                return;
            send("session.open", {
                session_id: id
            }, (r, success) => {
                if (!success)
                    return;
                if (r.session_id !== id) {
                    const unsaved = !!dirty[id];
                    let d = Object.assign({}, drafts);
                    d[r.session_id] = d[id];
                    delete d[id];
                    drafts = d;
                    delete dirty[id];
                    if (unsaved) {
                        dirty[r.session_id] = true;
                        saveTimer.restart();
                    }
                    sessionActivated(id, r.session_id);
                }
                loadHistory(r.session_id);
            });
        });
    }
    function submit(id) {
        if (isBusy(id))
            return;
        let f = Object.assign({}, inFlight);
        f[id] = true;
        inFlight = f;
        saveDraft(id, ok => {
            if (!ok) {
                let x = Object.assign({}, inFlight);
                delete x[id];
                inFlight = x;
                return;
            }
            send("session.prompt", {
                session_id: id
            }, (r, success) => {
                let x = Object.assign({}, inFlight);
                delete x[id];
                inFlight = x;
                loadHistory(id);
                if (success)
                    message("Turn ended: " + (r.stopReason || "complete") + ". The draft is retained.", false);
            });
            loadHistory(id);
        });
    }
    function listSessions(cursor) {
        send("session.list", cursor ? {
            cursor: cursor
        } : {}, (r, ok) => {
            if (ok)
                nextCursor = r.nextCursor || "";
        });
    }
    function approvals(id) {
        return Object.keys(runtime.permissions || {}).filter(k => runtime.permissions[k].params.sessionId === id).map(k => ({
                    token: k,
                    request: runtime.permissions[k]
                }));
    }
    function modelChoices() {
        let result = [];
        (runtime.providers || []).forEach(p => {
            if (p.kind === "cloud" && runtime.mode !== "cloud")
                return;
            p.models.forEach(m => result.push({
                    label: m.label + (p.kind === "cloud" ? " · cloud" : ""),
                    provider: p.id,
                    model: m.id
                }));
        });
        return result;
    }
    function displayEvents(id) {
        const source = histories[id] || [];
        let result = [];
        source.forEach(e => {
            const d = e.data || {};
            if (e.kind === "prompt_intent") {
                const blocks = (d.prompt || []).filter(p => p.type === "text");
                result.push({
                    kind: "You",
                    text: (fullDetail ? blocks : blocks.slice(0, 1)).map(p => p.text).join("\n"),
                    detail: false
                });
                if (!fullDetail && blocks.length > 1)
                    result.push({
                        kind: "Context",
                        text: "Selected reference snapshots were included. Full trace shows the exact submitted data.",
                        detail: true
                    });
            } else if (e.kind === "update") {
                const type = d.sessionUpdate;
                if (type === "agent_message_chunk" || type === "agent_thought_chunk") {
                    const thought = type === "agent_thought_chunk";
                    if (thought && !fullDetail)
                        return;
                    const text = d.content && d.content.type === "text" ? d.content.text : JSON.stringify(d.content || {});
                    const kind = thought ? "Reasoning" : "Assistant";
                    const last = result[result.length - 1];
                    if (last && last.kind === kind)
                        last.text += text;
                    else
                        result.push({
                            kind: kind,
                            text: text,
                            detail: thought
                        });
                } else if (type === "tool_call" || type === "tool_call_update") {
                    const key = d.toolCallId;
                    let existing = result.find(r => r.toolId === key);
                    if (!existing) {
                        existing = {
                            kind: "Tool",
                            toolId: key,
                            text: "",
                            detail: true,
                            raw: {}
                        };
                        result.push(existing);
                    }
                    existing.raw = Object.assign(existing.raw, d);
                    existing.text = fullDetail ? JSON.stringify(existing.raw, null, 2) : (existing.raw.title || existing.raw.kind || "Tool action") + " · " + (existing.raw.status || "pending");
                } else if (fullDetail && type !== "usage_update")
                    result.push({
                        kind: "Event",
                        text: JSON.stringify(d, null, 2),
                        detail: true
                    });
            } else if (e.kind === "turn_result" || e.kind === "turn_interrupted") {
                result.push({
                    kind: e.kind === "turn_interrupted" ? "Interrupted" : "Turn",
                    text: JSON.stringify(d, null, 2),
                    detail: true
                });
            } else if (fullDetail && e.kind.indexOf("permission") === 0)
                result.push({
                    kind: "Permission",
                    text: JSON.stringify(d, null, 2),
                    detail: true
                });
        });
        return result;
    }
}
