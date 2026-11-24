import QtQuick
import Quickshell.Io

// Local user-session IPC. All writes go through the same service as the UI.
QtObject {
    id: api
    property var backend
    property int sequence: 0
    property var replies: ({})

    function finish(ticket, value) {
        let next = Object.assign({}, replies);
        next[ticket] = JSON.stringify(value);
        replies = next;
    }
    function request(command: string, argsJson: string): string {
        let ticket = String(++sequence);
        if (Object.keys(replies).length >= 32)
            return JSON.stringify({
                error: "Too many uncollected requests"
            });
        let next = Object.assign({}, replies);
        next[ticket] = null;
        replies = next;
        try {
            if (argsJson.length > (command === "inbox-chunk" ? 140000 : 32768))
                throw new Error("Arguments exceed the command size limit");
            let args = JSON.parse(argsJson || "{}");
            if (!args || Array.isArray(args) || typeof args !== "object")
                throw new Error("Arguments must be a JSON object");
            if (!backend.available || backend.closing || !backend.state.root)
                throw new Error("Soliton service is initializing, unavailable or closing");
            // MOBILE-SNAPSHOT-V1: read-only, one event-loop turn. Rust writes
            // still use the original guarded commands below. No draft contents,
            // notes, filesystem sources or agent capabilities are exposed.
            if (command === "desktop-capabilities") {
                finish(ticket,{ok:true,result:{protocol:1,history:{version:1,providers:['voxtype','codex','opencode','chatgpt','console','manual','web','youtube','browser','feed'],max_page:100},briefings:{version:1,read_only:true,commands:['briefing-list','briefing-status','briefing-chunk']},modules:{version:1,read_only:true,commands:['modules-list','modules-status']},proposals:{version:1,approval:'journaled',calendar_preview:true},inbox:{version:1,max_chunk_bytes:65536,max_file_bytes:1073741824,transport:'local-ipc',wire_encoding:'hex'},recovery:{version:1,scope:'explicit-file-group',max_files:256,filesystem_atomic:false},remote_bridge:'Capabilities require an authenticated bridge adapter; local IPC is not network authorization.'}});
            } else if (command === "mobile-snapshot") {
                finish(ticket, {
                    ok: true,
                    result: {
                        protocol: 1,
                        schema: backend.state.schema,
                        root: backend.state.root,
                        timezone: backend.prefs.timezone,
                        week_start: 0,
                        records: backend.records.filter(r => ["category", "goal", "task"].includes(r.kind)),
                        dirty_ids: Object.values(backend.drafts).filter(d => d.dirty && d.id && backend.records.some(r => r.id === d.id && ["category", "goal", "task"].includes(r.kind))).map(d => d.id)
                    }
                });
            } else if (command === "info") {
                let counts = {};
                backend.records.forEach(r => counts[r.kind] = (counts[r.kind] || 0) + 1);
                finish(ticket, {
                    ok: true,
                    result: {
                        protocol: 1,
                        root: backend.state.root,
                        counts: counts,
                        savedViews: backend.prefs.savedViews || [],
                        dirtyDrafts: Object.values(backend.drafts).filter(d => d.dirty).length
                    }
                });
            } else if (command === "list") {
                let records = backend.records.filter(r => (!args.kind || r.kind === args.kind) && (!args.goal_id || r.goal_id === args.goal_id) && (!args.category_id || r.category_id === args.category_id) && (!args.tag || (r.tags || []).includes(args.tag)) && (!args.status || r.status === args.status) && (args.archived === undefined || r.hidden === args.archived) && (!args.search || [r.title, r.body, (r.tags || []).join(" ")].join(" ").toLowerCase().includes(args.search.toLowerCase()))).sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));
                let offset = Math.max(0, Math.floor(args.offset || 0)), limit = Math.max(1, Math.min(100, Math.floor(args.limit || 50)));
                let items = records.slice(offset, offset + limit);
                if (!args.full)
                    items = items.map(r => ({
                                id: r.id,
                                kind: r.kind,
                                title: r.title,
                                hash: r.hash,
                                status: r.status,
                                priority: r.priority,
                                goal_id: r.goal_id,
                                category_id: r.category_id,
                                hidden: r.hidden,
                                tags: r.tags
                            }));
                finish(ticket, {
                    ok: true,
                    result: {
                        items: items,
                        total: records.length,
                        next: offset + limit < records.length ? offset + limit : null
                    }
                });
            } else if (command === "get") {
                let record = backend.get(args.id);
                if (!record)
                    throw new Error("Object not found");
                finish(ticket, {
                    ok: true,
                    result: record
                });
            } else if (command === "show") {
                if (args.view && !["Categories & Goals", "Tasks", "Calendar", "Notes", "Strategy", "Goals", "Dashboard", "Learning", "Settings", "Portal"].includes(args.view))
                    throw new Error("Unknown view");
                backend.navigateView(args.view || "Categories & Goals");
                if (args.search !== undefined)
                    backend.search = String(args.search);
                if (args.branches !== undefined)
                    backend.branches = args.branches;
                if (args.status !== undefined)
                    backend.status = args.status;
                if (args.priority !== undefined)
                    backend.priority = args.priority;
                if (args.archived !== undefined)
                    backend.showArchived = args.archived;
                if (args.id) {
                    if (!backend.get(args.id))
                        throw new Error("Object not found");
                    backend.select(args.id);
                }
                finish(ticket, {
                    ok: true,
                    result: {
                        view: backend.view,
                        selectedId: backend.selectedId
                    }
                });
            } else if (command === "save-view") {
                if (!args.name || typeof args.name !== "string")
                    throw new Error("Name required");
                backend.saveView(args.name);
                finish(ticket, {
                    ok: true,
                    result: {
                        name: args.name
                    }
                });
            } else {
                if (!["reorder", "save", "complete", "occurrence", "history", "restore", "duplicate", "promote", "calendar", "export", "history-list", "history-detail", "history-revisions", "history-discover", "history-capture-bootstrap", "history-capture-sync", "history-import", "history-import-batch", "briefing-list", "briefing-status", "briefing-chunk", "modules-list", "modules-status", "inbox-destinations", "inbox-begin", "inbox-status", "inbox-chunk", "inbox-commit", "inbox-cancel", "inbox-list", "proposal-list", "proposal-detail", "proposal-calendar"].includes(command))
                    throw new Error("Unsupported command");
                if (args.id && !["history", "calendar"].includes(command) && Object.values(backend.drafts).some(d => d.id === args.id && d.dirty))
                    throw new Error("This object has an unsaved UI draft. Save or resolve it before an agent edits it.");
                backend.send(command, args, (value, ok, error) => finish(ticket, {
                        ok: ok,
                        result: value,
                        error: ok ? undefined : (error || backend.notice)
                    }));
            }
        } catch (error) {
            finish(ticket, {
                ok: false,
                error: String(error)
            });
        }
        return JSON.stringify({
            ticket: ticket
        });
    }
    function read(ticket: string, offset: int): string {
        if (!(ticket in replies))
            return JSON.stringify({
                error: "Unknown ticket"
            });
        if (replies[ticket] === null)
            return JSON.stringify({
                pending: true
            });
        let text = replies[ticket], start = Math.max(0, offset);
        let end = Math.min(text.length, start + 8192);
        if (end < text.length && text.charCodeAt(end - 1) >= 0xD800 && text.charCodeAt(end - 1) <= 0xDBFF)
            --end;
        return JSON.stringify({
            chunk: text.slice(start, end),
            next: end < text.length ? end : null
        });
    }
    function forget(ticket) {
        let next = Object.assign({}, replies);
        // Pending requests must still have somewhere to deliver their result.
        if (next[ticket] !== null)
            delete next[ticket];
        replies = next;
    }
    property IpcHandler ipc: IpcHandler {
        target: "soliton-api"
        function request(command: string, argsJson: string): string { return api.request(command, argsJson); }
        function read(ticket: string, offset: int): string { return api.read(ticket, offset); }
        function forget(ticket: string) { api.forget(ticket); }
    }
}
