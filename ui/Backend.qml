pragma Singleton
import QtQuick
import Quickshell
import Quickshell.Io
import "CalendarMath.js" as Cal
import "Routes.js" as Routes

QtObject {
    id: b

    property IpcHandler acceptance: IpcHandler {
        function create(kind: string) {
            b.create(kind);
        }
        function date(date: string) {
            b.selectedDate = date;
        }
        function edit(key: string, valueJson: string) {
            b.edit(key, JSON.parse(valueJson));
        }
        function mode(mode: string) {
            b.calendarMode = mode;
        }
        function disconnect() {
            b.disconnectService();
        }
        function reconnect() {
            b.reconnect();
        }
        function navigate(page: string, view: string) {
            b.navigate(page, view);
        }
        function detailLayout(full: bool) { b.detailFull = full; }
        // The second flag opened a retired pane; callers still pass it.
        function panels(console: bool, retired: bool) {
            b.consoleOpen = console;
        }
        function chooseTool(tool: string) { b.chooseTool(tool); }
        function request(command: string, argsJson: string): int {
            var id = ++b.sequence;
            b.send(command, JSON.parse(argsJson), (r, ok) => {
                var a = Object.assign({}, b.acceptanceResults);
                a[id] = {
                    result: r,
                    ok: ok
                };
                b.acceptanceResults = a;
            });
            return id;
        }
        function result(id: string): string {
            return JSON.stringify(b.acceptanceResults[id] || null);
        }
        function filter(priority: string, status: string, search: string) {
            b.priority = priority;
            b.status = status;
            b.search = search;
            b.remember();
        }
        function branch(id: string) {
            b.toggleBranch(id);
        }
        function clearFilters() {
            b.clearFilters();
        }
        function saveView(name: string) {
            b.saveView(name);
        }
        function applySavedScope(index: int) {
            if (b.prefs.savedViews && b.prefs.savedViews[index])
                b.applyScope(b.prefs.savedViews[index]);
        }
        function save() {
            b.saveDraft();
        }
        function select(id: string) {
            b.select(id);
        }
        function setting(key: string, valueJson: string) {
            b.setting(key, JSON.parse(valueJson));
        }
        function snapshot(): string {
            return JSON.stringify({
                dataRoot: b.state.root,
                showCompleted: b.showCompleted,
                detailFull: b.detailFull,
                page: b.page,
                view: b.view,
                selectedId: b.selectedId,
                activeKey: b.activeKey,
                activeDraft: b.activeDraft ? {
                    id: b.activeDraft.id,
                    kind: b.activeDraft.kind,
                    title: b.activeDraft.title,
                    revision: b.activeDraft.revision || 1,
                    status: b.activeDraft.status,
                    scheduled_for: b.activeDraft.scheduled_for,
                    hash: b.activeDraft.hash,
                    dirty: !!b.activeDraft.dirty
                } : null,
                branches: b.branches,
                priority: b.priority,
                status: b.status,
                search: b.search,
                savedViews: (b.prefs.savedViews || []).map(v => v.name),
                filteredTasksCount: b.tasks.length,
                filteredEventsCount: b.filteredEvents.length,
                eventsCount: b.events.length,
                available: b.available,
                recordsCount: b.records.length,
                notice: b.notice
            });
        }
        function summary(): string {
            return JSON.stringify({
                count: b.records.length,
                notice: b.notice
            });
        }

        enabled: Quickshell.env('SOLITON_AUTOMATION') === '1'
        target: 'soliton'
    }
    property var acceptanceResults: ({})
    property var activeDraft: drafts[activeKey] || null
    property string activeKey: ''
    property AgentApi agentApi: AgentApi {
        backend: b
    }
    property bool available: false
    property bool board: false
    property var branches: []
    property Process bridge: Process {
        command: [Quickshell.env('SOLITON_BIN'), Quickshell.env('SOLITON_DATA')]
        running: true
        stdinEnabled: true

        stderr: SplitParser {
            onRead: data => {
                b.serviceError = data;
                b.message(data, true);
            }
        }
        stdout: SplitParser {
            onRead: data => {
                try {
                    var r = JSON.parse(data);
                    if (r.state)
                        b.install(r.state);
                    if (!r.ok && !b.quietErrors[r.id])
                        b.message(r.error, true);
                    delete b.quietErrors[r.id];
                    var cb = b.callbacks[r.id];
                    delete b.callbacks[r.id];
                    if (cb) {
                        try { cb(r.result, r.ok, r.error); }
                        catch (callbackError) { console.warn('Soliton view callback failed:', callbackError); }
                    }
                } catch (e) {
                    b.message('Could not decode service response: ' + e, true);
                }
            }
        }

        // No exit arguments are used; connect the callback without compiling Qt's unexported QProcess::ExitStatus type.
        Component.onCompleted: b.bridge.exited.connect(() => {
            b.available = false;
            b.message(b.serviceError || 'Local service stopped. Saved objects are on disk; unsaved text remains in this window.', true);
            var c = b.callbacks;
            b.callbacks = ({});
            Object.keys(c).forEach(k => { try { c[k](null, false, 'Local service stopped'); } catch (error) { console.warn('Soliton closed-view callback:', error); } });
            if(b.activeHistoryJob){var j=b.activeHistoryJob;b.activeHistoryJob=null;b.historyPolling=false;b.historyCallback(j,null,false,'Service restarted. Retry to reconcile archived content.');}
            b.historyStarting=false;
            var waiting=b.historyRequests;b.historyRequests=[];
            waiting.forEach(j=>b.historyCallback(j,null,false,'Local service stopped. Retry when connected.'));
        })
        onStarted: {
            b.available = true;
            b.send('state', {}, (r, ok) => {
                if (ok) {
                    b.install(r);
                    b.refreshTools();
                    b.bootstrapCapture();
                }
            });
        }
    }
    property string calendarMode: 'Week'
    property Timer calendarTimer: Timer {
        interval: 100

        onTriggered: b.loadCalendar()
    }
    property var callbacks: ({})
    property var quietErrors: ({})
    property var categories: records.filter(r => r.kind === 'category' && !r.hidden).sort((a, c) => a.order - c.order)
    property Timer closeWait: Timer {
        interval: 50
        repeat: true

        onTriggered: {
            if (b.saving || b.prefsSaving || b.activeHistoryJob || b.historyStarting || b.historyRequests.length)
                return;
            stop();
            b.flushPreferences(ok => {
                if (ok)
                    b.closeReady();
                else {
                    b.closing = false;
                    b.message('Close held: workspace drafts could not be saved.', true);
                }
            });
        }
    }
    property bool closing: false
    property string compactPane: 'work'
    property bool consoleOpen: false
    property bool detailFull: false
    property bool detailsOpen: false
    property string detailRoute: ''
    property var routeContexts: ({})
    property var routeViews: ({})
    property bool restoringRoute: false
    readonly property string routeKey: page + '/' + view
    readonly property bool hasRouteDetails: detailsOpen && !!activeDraft && detailRoute === routeKey && Routes.supportsDetails(page, view)
    signal routeLeaving
    // The right side holds one module at a time; one more can live in its own window.
    // The console flag covers the launcher and the terminal tools and follows slotModule.
    property string poppedTool: ''
    property string slotModule: ''
    readonly property string slotTool: slotModule
    readonly property var moduleNames: ({ launcher: 'Tools', terminal: 'Terminal', codex: 'Codex', qream: 'QREAM', zeno: 'Zeno' })
    readonly property var terminalTools: ['terminal', 'codex', 'qream', 'zeno']
    // Tools present on this machine, as the engine's tool-available reports them.
    property var toolsAvailable: []
    property bool toolsKnown: false
    // The last launch of each tool from this window: folder, started, message.
    property var toolLaunches: ({})
    readonly property var tools: terminalTools.filter(t => toolsAvailable.indexOf(t) >= 0)
    property bool slotSyncing: false
    function isConsoleModule(t) { return t === 'launcher' || terminalTools.indexOf(t) >= 0; }
    function openTool(t) {
        // The console shortcut and palette entry open the launcher.
        if (t === 'console') t = 'launcher';
        if (t && t === poppedTool) t = isConsoleModule(t) ? 'launcher' : slotModule;
        slotModule = t || '';
        slotSyncing = true;
        consoleOpen = isConsoleModule(slotModule);
        slotSyncing = false;
        compactPane = slotModule ? 'console' : 'work';
        remember();
    }
    function cycleTool() {
        var free = tools.filter(t => t !== poppedTool);
        if (!free.length) return;
        openTool(free[(free.indexOf(slotModule) + 1) % free.length]);
    }
    function popOutTool() {
        var t = slotModule;
        if (!t || t === 'launcher' || poppedTool) return;
        openTool('');
        poppedTool = t;
    }
    function refreshTools() {
        send('tool-available', {}, (r, ok) => {
            if (!ok || !r) return;
            toolsAvailable = (r.tools || []).filter(x => x.available).map(x => x.tool);
            toolsKnown = true;
        }, true);
    }
    // The kind of object each view shows; a selection left over from another view is not this view's.
    function selectionShown(s) {
        if (!s || s.hidden) return false;
        switch (s.kind) {
        case 'goal': return page === 'Goals';
        case 'note': return page === 'Notes';
        case 'learning': return page === 'Learning' && view === 'Notes';
        case 'strategy': return page === 'Strategy' && view === 'Ideas';
        case 'task': return page === 'Tasks' || page === 'Dashboard';
        default: return false;
        }
    }
    // The folder the engine resolves: the page, the view and the object selected in it.
    function toolContext() {
        return { page: page, view: view, selected_id: selectionShown(selected) ? selectedId : '' };
    }
    function launchTool(t) {
        var args = { tool: t };
        if (t !== 'qream') args.context = toolContext();
        send('tool-launch', args, (r, ok) => {
            if (!ok || !r) return;
            var next = Object.assign({}, toolLaunches);
            next[t] = { started: !!r.started, folder: String(r.folder || ''), message: String(r.message || '') };
            toolLaunches = next;
            if (!r.started) message(moduleNames[t] + ' could not be opened: ' + (terminalTools.indexOf(t) >= 0 && !toolsAvailable.includes(t) ? 'it is not installed on this machine.' : 'no terminal is installed.'), true);
        });
    }
    // A launcher entry: the module takes the slot; a terminal tool also opens its own window.
    function chooseTool(t) {
        openTool(t);
        if (terminalTools.indexOf(t) >= 0) launchTool(t);
    }
    property var drafts: ({})
    property var draggingCard: null
    property var events: []
    property var filteredEvents: events.filter(e => matches(e))
    property bool focusMode: false
    property bool goalList: false
    property var goals: records.filter(r => r.kind === 'goal' && !r.hidden).sort((a, c) => a.order - c.order)
    // The minute tick reads the clock only. Data changes arrive as the engine's
    // state events (install refreshes the calendar); the year-scale projections
    // are asked again only when the local date rolls over.
    property Timer heartbeat: Timer {
        interval: 60000
        repeat: true
        running: b.available && b.restored

        onTriggered: b.tickClock()
    }
    function tickClock() {
        var day = today;
        send('clock', { timezone: prefs.timezone }, (r, ok) => {
            if (!ok || !r) return;
            nowLocal = r.local;
            if (today !== day) refreshCalendar();
        }, true);
    }
    property var history: []
    property bool navCollapsed: false
    property string notice: 'Opening your workspace…'
    property bool noticeError: false
    property Timer noticeTimer: Timer {
        interval: 4500

        onTriggered: if (!b.noticeError)
            b.notice = ''
    }
    property string nowLocal: Qt.formatDateTime(new Date(), "yyyy-MM-dd'T'HH:mm")
    property string page: 'Dashboard'
    property Timer prefTimer: Timer {
        interval: 500

        onTriggered: if (b.prefsDirty)
            b.flushPreferences()
    }
    property var prefs: ({
            timezone: 'America/Toronto',
            startHour: 5,
            endHour: 24,
            hourHeight: 58,
            weekStart: 0,
            savedViews: [],
            // Opening Soliton does no work: capture and the local model run
            // only after he turns them on in Settings.
            automaticCapture: false,
            localModel: false
        })
    // The Settings theme choice lives with the other workspace preferences.
    onPrefsChanged: {
        Theme.choice = typeof prefs.theme === 'string' ? prefs.theme : ''
        if ((prefs.localModel === true) !== modelAsked) { modelAsked = prefs.localModel === true; refreshModelChosen() }
    }
    // Whether he has chosen a model for analysis. With Local model on and none
    // chosen, every analysis waits for his model; the views say so.
    property bool modelChosen: false
    property bool modelAsked: false
    function refreshModelChosen() {
        if (prefs.localModel !== true) { modelChosen = false; return }
        send('briefing-model', {}, (r, ok) => { b.modelChosen = ok && !!r && r.set === true }, true)
    }
    property bool prefsDirty: false
    // Private mode is the workspace setting privateMode: read with the
    // workspace at start, written through preferences. The engine filters
    // every read by it, so a change reloads what is on screen at once.
    readonly property bool privateMode: !!prefs.privateMode
    property bool privateSaved: false
    signal privacyApplied()
    property Timer privateRetry: Timer {
        interval: 60
        onTriggered: b.applyPrivateMode()
    }
    function setPrivateMode(on) {
        var p = Object.assign({}, prefs);
        p.privateMode = !!on;
        prefs = p;
        applyPrivateMode();
    }
    function togglePrivateMode() {
        setPrivateMode(!privateMode);
    }
    function applyPrivateMode() {
        if (!available) { privateRetry.restart(); return; }
        var wanted = privateMode;
        flushPreferences(ok => {
            if (!ok) { privateRetry.restart(); return; }
            if (wanted !== privateMode) { applyPrivateMode(); return; }
            refreshPrivateViews();
        });
    }
    function refreshPrivateViews() {
        send('state', {}, (r, ok) => {
            if (ok) install(r);
            if (ok && privateMode) {
                // A hidden item leaves the window's own copies too: its pane
                // closes and its unchanged draft is dropped.
                var shown = {};
                (r.records || []).forEach(x => shown[x.id] = true);
                if (activeDraft && activeDraft.id && !shown[activeDraft.id]) closeDetails();
                var kept = {}, dropped = false;
                Object.keys(drafts).forEach(k => {
                    var d = drafts[k];
                    if (d && d.id && !shown[d.id] && !d.dirty) dropped = true;
                    else kept[k] = d;
                });
                if (dropped) drafts = kept;
                if (selectedId && !shown[selectedId]) selectedId = '';
            }
            privateSaved = privateMode;
            privacyApplied();
        });
        if (habitsLoaded || habitsReadying) loadHabits();
        hearthReload();
        refreshCalendar();
        refreshProposals();
    }
    // The private marks as the engine holds them. A preferences reply is
    // never filtered, and a state read is only while private mode is off, so
    // the marks are learned from those two and written only from here.
    property var privateMarks: []
    property var privateModuleMarks: []
    // The ledger's own module marks (the finance mark the migration wrote),
    // as the Settings read reports them (T028).
    property var ledgerModuleMarks: []
    property bool ledgerMarksReported: false
    function isLedgerModulePrivate(key) { return ledgerModuleMarks.indexOf(key) >= 0; }
    function setLedgerModulePrivate(key, on) {
        send(on ? 'ledger-mark-private' : 'ledger-unmark-private', {scope: 'module', key: key}, (r, ok) => {
            if (!ok) { ledgerModuleMarks = ledgerModuleMarks.slice(); message('Could not save the private mark', true); return; }
            send('preferences', {privateMode: privateMode}, (p, pok) => { if (pok) learnPrivateMarks(p, true); if (privateMode) refreshPrivateViews(); }, true);
        }, true);
    }
    property bool privateMarksKnown: false
    function learnPrivateMarks(ui, unfiltered) {
        if (!ui || !(unfiltered || ui.privateMode === false || ui.privateMode === undefined)) return;
        privateMarks = (ui.privateIds || []).slice();
        privateModuleMarks = (ui.privateModules || []).slice();
        ledgerModuleMarks = (ui.ledgerPrivateModules || []).slice();
        ledgerMarksReported = Array.isArray(ui.ledgerPrivateModules);
        privateMarksKnown = true;
    }
    function isMarkedPrivate(id) {
        return !!id && privateMarks.indexOf(id) >= 0;
    }
    function isModulePrivate(key) {
        return privateModuleMarks.indexOf(key) >= 0;
    }
    function writePrivateMark(key, list) {
        var args = {replaceMarks: true};
        args[key] = list;
        send('preferences', args, (r, ok) => {
            if (ok) {
                learnPrivateMarks(r, true);
                if (privateMode) refreshPrivateViews();
            } else {
                // Republish so a check box bound to the old list returns.
                privateMarks = privateMarks.slice();
                privateModuleMarks = privateModuleMarks.slice();
                message('Could not save the private mark', true);
            }
        }, true);
    }
    function withPrivateMarks(action) {
        if (privateMarksKnown) { action(); return; }
        // Started with the mode on: the state read was filtered. Learn the
        // marks from a reply that is not, by sending the mode as it stands.
        send('preferences', {privateMode: privateMode}, (r, ok) => {
            if (!ok) { message('Could not save the private mark', true); return; }
            learnPrivateMarks(r, true);
            action();
        }, true);
    }
    function setMarkedPrivate(id, on) {
        if (!id) return;
        withPrivateMarks(() => {
            var list = privateMarks.filter(v => v !== id);
            if (on) list.push(id);
            writePrivateMark('privateIds', list);
        });
    }
    function setModulePrivate(key, on) {
        withPrivateMarks(() => {
            var list = privateModuleMarks.filter(v => v !== key);
            if (on) list.push(key);
            writePrivateMark('privateModules', list);
        });
    }
    property bool prefsSaving: false
    property string priority: 'all'
    property var records: state.records || []
    // Which build runs and whether it is the build its release record names.
    property var release: state.release || ({})
    property bool restored: false
    property string developerSection: 'Stack'
    property string developerModule: 'zetl'
    property string strategyTab: 'Ideation'
    property string historySelectedId: ''
    // Explicit navigation is a request, separate from mutable remembered selection.
    property var historyNavigation: ({serial:0, id:''})
    property var proposalDraft: ({})
    property var proposals: []
    property var proposalEvents: []
    property int proposalOffset: 0
    property int proposalLimit: 25
    property int proposalTotal: 0
    property bool proposalLoading: false
    property string proposalStatus: 'all'
    property bool saving: false
    property string search: ''
    // Shared task visibility preference. Leaf pages bind their task models to
    // this persisted value so Dashboard, Tasks and task filters stay in sync.
    property bool showCompleted: false
    property var selected: get(selectedId)
    property string selectedDate: Cal.iso(new Date())
    property string selectedId: ''
    property int sequence: 0
    property string serviceError: ''
    property bool showArchived: false
    property var state: ({
            records: [],
            ui: {}
        })
    property string status: 'all'
    // The tracker store's revision from the engine's state event; null while no sheet view watches it.
    readonly property var trackerRevision: state.trackerRevision === undefined ? null : state.trackerRevision
    // How many views want the store watched (the sheet, a record's row line); the engine watches while any does.
    property int trackerWatchers: 0
    function watchTracker(on) {
        var before = trackerWatchers;
        trackerWatchers = Math.max(0, trackerWatchers + (on ? 1 : -1));
        if ((before === 0) !== (trackerWatchers === 0))
            send('tracker-watch', {on: trackerWatchers > 0}, (r, ok) => {
                if (ok && trackerWatchers > 0) state = Object.assign({}, state, {trackerRevision: r.revision});
            }, true);
    }
    // A task from a tracker row: titled from the row's purpose, its source the row (a row:<id> tag), saved and opened.
    function taskFromRow(row) {
        var id = String(row.id || ''), title = String(row.purpose || row.summary || row.title || id).trim();
        if (!id) return;
        var d = {kind: 'task', id: '', title: title, body: '', category_id: '', goal_id: '', status: 'todo',
                 priority: 'medium', scheduled_for: '', due_on: '', target_on: '', recurrence: 'none', interval: 1,
                 repeat_until: '', color: '#c8a96b', links: [], files: [], tags: ['row:' + id], schedule: null};
        send('save', d, (r, ok) => {
            if (!ok) return;
            navigate('Dashboard', 'Timeline');
            select(r.id);
            message('Saved · ' + r.title, false);
        });
    }
    property var tasks: records.filter(r => r.kind === 'task' && matches(r)).sort((a, c) => a.order - c.order)
    property string today: nowLocal.substring(0, 10)
    property var todayEvents: []
    property string view: 'Timeline'

    signal closeReady
    signal paletteRequested
    // A category row of the side panel asks the Goals page for its actions menu.
    signal categoryActionsRequested(var record, Item item)
    signal composeProposalRequested
    signal reviewProposalRequested(string proposalId)

    function beginProposal(record, segment) {
        if (proposalDraft.source_id && proposalDraft.title) {
            message('Your unfinished proposal is preserved. Save or discard it before starting another source.');
        } else {
            proposalDraft = {source_id:record.id,source_revision:String(record.revision),segment_id:segment.id,source_title:record.title,excerpt:segment.text,title:'',rationale:'',goal_id:'',date:selectedDate,time:'',duration_minutes:30};
            remember();
        }
        composeProposalRequested();
    }
    function refreshProposals(offset, status) {
        if (typeof offset !== 'number')
            offset = 0;
        if (typeof status !== 'string' || !status)
            status = proposalStatus || 'all';
        proposalOffset = Math.max(0, Math.floor(offset));
        proposalStatus = status;
        if (!available)
            return;
        proposalLoading = true;
        send('proposal-list',{status:proposalStatus,offset:proposalOffset,limit:proposalLimit},(r,ok,e)=>{
            proposalLoading = false;
            if(ok){
                proposals=r.items || [];
                proposalTotal=Number(r.total || 0);
                refreshProposalCalendar();
            } else if(e) {
                message(e, true);
            }
        },true);
    }
    function showHistory(id) {
        historyNavigation = {serial:Number(historyNavigation.serial || 0)+1, id:String(id)};
        historySelectedId=String(id);setting('historyDetailFocused', true);strategyTab='History';navigate('Strategy', 'History');
    }

    function applyView(v) {
        navigate(v.page || page, v.view || view);
        branches = v.branches ? v.branches.slice() : [];
        priority = v.priority || 'all';
        status = v.status || 'all';
        search = v.search || '';
        board = v.board !== undefined ? v.board : board;
        calendarMode = v.calendarMode || calendarMode;
        if (v.showArchived !== undefined)
            showArchived = !!v.showArchived;
        remember();
    }
    function applyScope(v) {
        branches = v.branches ? v.branches.slice() : [];
        priority = v.priority || 'all';
        status = v.status || 'all';
        search = v.search || '';
        if (v.showArchived !== undefined)
            showArchived = !!v.showArchived;
        remember();
    }
    function branchMatches(r, seen) {
        if (!r || seen.indexOf(r.id) >= 0)
            return false;
        var catId = (r.goal_id && get(r.goal_id)) ? get(r.goal_id).category_id : r.category_id;
        if (branches.some(id => id === catId || id === r.goal_id || id === r.id))
            return true;
        return (r.links || []).some(id => branchMatches(get(id), seen.concat([r.id])));
    }
    function clearFilters() {
        branches = [];
        priority = 'all';
        status = 'all';
        search = '';
        showArchived = false;
        remember();
    }
    function color(id) {
        return (get(id) || {}).color || Theme.accent;
    }
    function complete(r, done) {
        var event = r;
        if (r.recurrence !== 'none' && !r.occurrence) {
            event = todayEvents.find(e => e.id === r.id && e.occurrence && e.date === today);
            if (!event) {
                select(r.id);
                message('Select an occurrence in the calendar to complete repeating work.');
                if (done) done(null, false, 'Select a recurring occurrence');
                return;
            }
        }
        send('complete', {
            id: r.id,
            hash: (get(r.id) || r).hash,
            occurrence: event.occurrence || '',
            occurrence_date: event.occurrence_date || ''
        }, (v, ok) => {
            if (ok) {
                var map = Object.assign({}, drafts);
                if (map[r.id] && !map[r.id].dirty)
                    delete map[r.id];
                drafts = map;
                if (!done) message('Completion updated');
            }
            if (done) done(v, ok);
        });
    }
    function create(kind, goalId, categoryId, day, time) {
        selectedId = '';
        activeKey = 'new:' + kind;
        var d = {
            kind: kind,
            id: '',
            title: '',
            body: '',
            category_id: categoryId || '',
            goal_id: goalId || '',
            status: kind === 'task' ? 'todo' : 'active',
            priority: 'medium',
            scheduled_for: day || '',
            due_on: '',
            target_on: '',
            recurrence: 'none',
            interval: 1,
            repeat_until: '',
            color: '#c8a96b',
            links: [],
            files: [],
            tags: [],
            dirty: true,
            schedule: time ? {
                time: time,
                duration_minutes: 60,
                timezone: prefs.timezone,
                fold: ''
            } : null
        };
        if (drafts[activeKey] && drafts[activeKey].dirty && drafts[activeKey].title) {
            d = drafts[activeKey];
            message('Your unfinished ' + kind + ' draft has been restored.');
        }
        var map = Object.assign({}, drafts);
        map[activeKey] = d;
        drafts = map;
        detailsOpen = true;
        detailRoute = routeKey;
        compactPane = 'detail';
        remember();
    }
    function edit(key, value) {
        if (!activeDraft)
            return;
        var d = Object.assign({}, activeDraft);
        d[key] = value;
        if (key === 'scheduled_for' && d.recurrence !== 'none' && !d.occurrence)
            d.repeat_start = value;
        d.dirty = true;
        var map = Object.assign({}, drafts);
        map[activeKey] = d;
        drafts = map;
        remember();
    }
    function editSchedule(key, value) {
        var z = Object.assign({
            time: '09:00',
            duration_minutes: 60,
            timezone: prefs.timezone,
            fold: ''
        }, activeDraft.schedule || {});
        z[key] = value;
        edit('schedule', z);
    }
    function flushPreferences(after) {
        if (prefsSaving) {
            prefTimer.restart();
            if (after)
                after(false);
            return;
        }
        var snapshot = preferenceState();
        prefsSaving = true;
        send('preferences', snapshot, (r, ok) => {
            prefsSaving = false;
            if (ok) learnPrivateMarks(r, true);
            if (ok && JSON.stringify(snapshot) === JSON.stringify(preferenceState()))
                prefsDirty = false;
            if (!ok) {
                var fixed = Object.assign({}, prefs);
                ['timezone', 'startHour', 'endHour', 'hourHeight', 'weekStart', 'reducedMotion'].forEach(k => fixed[k] = state.ui[k]);
                prefs = fixed;
                prefsDirty = true;
                prefTimer.restart();
            }
            if (after)
                after(ok);
        });
    }
    function get(id) {
        return records.find(r => r.id === id) || null;
    }
    function install(next) {
        state = next;
        learnPrivateMarks(next.ui, false);
        if (restored) {
            var nextRecords = next.records || [];
            var updated = Object.assign({}, drafts);
            var changed = false;
            Object.keys(updated).forEach(k => {
                var d = updated[k];
                var r = nextRecords.find(x => x.id === d.id);
                if (r && !d.dirty && r.hash !== d.hash) {
                    updated[k] = d.occurrence ? Object.assign({}, d, {
                        hash: r.hash,
                        revision: r.revision
                    }) : Object.assign({}, r, {
                        dirty: false
                    });
                    changed = true;
                }
            });
            if (changed)
                drafts = updated;
        }

        if (!restored) {
            var u = next.ui || {};
            prefs = Object.assign({}, prefs, u, { weekStart: 0 });
            var route = Routes.normalize(u.page || page, u.view || view);
            page = route.page;
            view = route.view;
            routeContexts = u.routeContexts || {};
            routeViews = u.routeViews || {};
            calendarMode = u.calendarMode || calendarMode;
            selectedDate = u.selectedDate || selectedDate;
            selectedId = u.selectedId || '';
            activeKey = u.activeKey || '';
            drafts = u.drafts || {};
            detailsOpen = !!u.detailsOpen;
            detailRoute = u.detailRoute || (Routes.supportsDetails(page, view) ? routeKey : '');
            detailFull = !!u.detailFull && detailRoute === routeKey;
            compactPane = detailsOpen && detailRoute === routeKey ? 'detail' : 'work';
            // A saved module that no longer exists restores to none.
            var savedModule = u.slotModule !== undefined ? u.slotModule : u.consoleOpen ? 'launcher' : '';
            slotModule = isConsoleModule(savedModule) ? savedModule : '';
            slotSyncing = true;
            consoleOpen = isConsoleModule(slotModule);
            slotSyncing = false;
            navCollapsed = !!u.navCollapsed;
            branches = u.branches || [];
            search = u.search || '';
            priority = u.priority || 'all';
            status = u.status || 'all';
            board = !!u.board;
            goalList = !!u.goalList;
            showCompleted = u.showCompleted !== undefined ? !!u.showCompleted : false;
            showArchived = !!u.showArchived;
            developerSection = u.developerSection || 'Stack';
            developerModule = u.developerModule || 'zetl';
            strategyTab = u.strategyTab || 'Ideation';
            historySelectedId = u.historySelectedId || '';
            proposalDraft = u.proposalDraft || {};
            restored = true;
            message(next.recovered ? 'Recovered ' + next.recovered + ' interrupted changes.' : 'Your workspace is ready.', false);
        }
        refreshCalendar();
        refreshProposals();
    }
    function label(id) {
        return (get(id) || {}).title || 'Unassigned';
    }
    function loadCalendar() {
        var d = Cal.parse(selectedDate), year = d.getFullYear();
        send('calendar', {
            from: (year - 1) + '-12-20',
            through: (year + 1) + '-01-10',
            timezone: prefs.timezone
        }, (r, ok) => {
            if (ok)
                events = r;
        });
        send('clock', {
            timezone: prefs.timezone
        }, (r, ok) => {
            if (ok) {
                nowLocal = r.local;
                send('calendar', {
                    from: today,
                    through: today,
                    timezone: prefs.timezone
                }, (items, valid) => {
                    if (valid)
                        todayEvents = items;
                });
            }
        });
    }
    function matches(r) {
        if (showArchived ? !r.hidden : r.hidden)
            return false;
        if (branches.length && !branchMatches(r, []))
            return false;
        if (priority !== 'all' && r.priority !== priority)
            return false;
        if (status !== 'all' && Cal.effectiveStatus(b, r) !== status)
            return false;
        return !search || [r.title, r.body, r.goal_title, r.category_title, (r.tags || []).join(' ')].join(' ').toLowerCase().indexOf(search.toLowerCase()) >= 0;
    }
    function message(text, error) {
        notice = text;
        noticeError = !!error;
        noticeTimer.restart();
    }
    function moveEvent(e, day, minute, duration) {
        var r = get(e.id);
        if (!r)
            return;
        var z = {
            time: Cal.clock(minute),
            duration_minutes: duration || ((e.schedule || {}).duration_minutes) || 60,
            timezone: prefs.timezone,
            fold: (e.schedule || {}).fold || 'earlier'
        };
        if (e.occurrence)
            send('occurrence', {
                id: r.id,
                hash: r.hash,
                occurrence: e.occurrence,
                occurrence_date: e.occurrence_date,
                date: day,
                schedule: z
            }, (v, ok) => {
                if (ok)
                    message('Occurrence rescheduled');
            });
        else
            mutate(r, {
                scheduled_for: day,
                schedule: z
            });
    }
    function moveGoalToCategory(r, category) {
        if (r.category_id === category)
            return;
        if (Object.values(drafts).some(d => d.id === r.id && d.dirty)) {
            message('Save or discard this card’s draft before moving it.', true);
            return;
        }
        mutate(r, {
            category_id: category,
            order: Math.max(0, ...goals.map(g => g.order)) + 1048576
        });
    }
    function moveOrder(r, delta) {
        var peers = records.filter(x => x.kind === r.kind && !x.hidden && (r.kind !== 'goal' || x.category_id === r.category_id)).sort((a, c) => a.order - c.order);
        var j = peers.findIndex(x => x.id === r.id) + delta;
        if (j >= 0 && j < peers.length)
            placeRecord(r, peers[j], delta > 0);
    }
    function moveStatus(r, status, done) {
        if (r.recurrence && r.recurrence !== 'none') {
            var event = Cal.statusRecord(b, r);
            if (!event.occurrence) {
                select(r.id);
                message('Select an occurrence in the calendar to change this recurring task’s status.', true);
                if (done) done(null, false);
                return;
            }
            send('occurrence', {id:r.id, hash:(get(r.id) || r).hash,
                occurrence:event.occurrence, occurrence_date:event.occurrence_date,
                status:status}, done);
            return;
        }
        mutate(r, {
            status: status
        }, done);
    }
    function moveToAllDay(e, day) {
        var r = get(e.id);
        if (!r)
            return;
        if (e.occurrence)
            send('occurrence', {
                id: r.id,
                hash: r.hash,
                occurrence: e.occurrence,
                occurrence_date: e.occurrence_date,
                date: day,
                schedule: null
            });
        else if (e.event_kind === 'due')
            mutate(r, {
                due_on: day
            });
        else
            mutate(r, {
                scheduled_for: day,
                schedule: null
            });
    }
    function moveToDate(e, day) {
        var r = get(e.id);
        if (!r)
            return;
        if (e.occurrence)
            send('occurrence', {
                id: r.id,
                hash: r.hash,
                occurrence: e.occurrence,
                occurrence_date: e.occurrence_date,
                date: day
            });
        else if (e.event_kind === 'due')
            mutate(r, {
                due_on: day
            });
        else
            mutate(r, {
                scheduled_for: day,
                schedule: r.schedule
            });
    }
    function mutate(r, patch, done) {
        send('save', Object.assign({
            id: r.id,
            hash: r.hash
        }, patch), (v, ok) => {
            if (ok) {
                var map = Object.assign({}, drafts);
                if (map[r.id] && !map[r.id].dirty)
                    map[r.id] = Object.assign({}, v, {
                        dirty: false
                    });
                drafts = map;
                if (!done) message('Saved · ' + v.title);
            }
            if (done) done(v, ok);
        });
    }

    // Habits (Hearth): every value below is read from or written through the
    // engine's habit-* calls; nothing is counted here.
    property var habitList: ({habits:[], bundles:{morning:[], evening:[]}})
    property var habitMap: ({habits:[]})
    property bool habitsLoaded: false
    property bool habitsReadying: false
    property string habitDay: ''
    readonly property string habitShownDay: habitDay || today
    property int habitSpan: 91
    readonly property bool habitHideAll: !!b.privateMode
    property bool habitFormOpen: false
    property string habitFormId: ''
    property int habitFormSerial: 0
    property var habitUndo: null
    property var habitFocusRequest: ({serial:0, bundle:''})
    // Voice note playback for the palette: the open note says when it can play;
    // a palette row asks it to act.
    property bool voicePlaybackReady: false
    property var voicePlaybackRequest: ({serial:0, action:''})
    function voicePlayback(action) { voicePlaybackRequest = {serial: voicePlaybackRequest.serial + 1, action: action}; }
    onHabitShownDayChanged: if (page === 'Hearth') loadHabits()
    onHabitSpanChanged: if (page === 'Hearth') loadHabitMap()
    onHabitHideAllChanged: if (page === 'Hearth') loadHabits()

    function habit(id) {
        return habitList.habits.find(h => h.id === id) || null;
    }
    function habitValue(id) {
        var row = habitMap.habits.find(h => h.id === id);
        var cell = row && row.days.length ? row.days[row.days.length - 1] : null;
        return cell && cell.day === habitShownDay ? cell.value : null;
    }
    function loadHabits() {
        send('habit-list', {day: habitShownDay, hide_all: habitHideAll}, (r, ok, error) => {
            if (!ok) {
                // The first opening readies the store as part of opening.
                if (/no ledger yet/.test(error || '') && !habitsReadying) {
                    habitsReadying = true;
                    send('ledger-migrate', {}, (m, migrated) => {
                        if (migrated) loadHabits();
                        else message('Habits are not available right now', true);
                    }, true);
                } else {
                    message('Habits are not available right now', true);
                }
                return;
            }
            habitList = r;
            loadHabitMap();
        }, true);
    }
    function loadHabitMap() {
        var through = habitShownDay;
        send('habit-heat-map', {from: Cal.add(through, 1 - habitSpan), through: through, hide_all: habitHideAll}, (r, ok) => {
            if (!ok) { message('Habits are not available right now', true); return; }
            if (r.through === habitShownDay) habitMap = r;
            habitsLoaded = true;
        }, true);
    }
    function habitCheckValue(h, value) {
        return h.direction === 'break' ? {count: Math.max(0, value || 0)} : {done: !!value};
    }
    // The day's current check-in as habit-list gives it for the shown day.
    function habitCheckInId(id) {
        var h = habit(id);
        return h && habitList.day === habitShownDay ? h.check_in || null : null;
    }
    // One check-in per habit and day: the first press writes it, any later one
    // corrects it, keeping both. Undo of a first entry takes it back; undo of a
    // correction is one more correction.
    function habitCheckIn(id, value, undoing) {
        var h = habit(id);
        if (!h) return;
        var day = habitShownDay, previous = habitValue(id), existing = habitCheckInId(id);
        var failed = () => { message('Could not save the check-in', true); loadHabits(); };
        var done = (r) => {
            habitUndo = undoing ? null
                : existing ? {kind: 'check-in', id: id, day: day, value: previous}
                : {kind: 'retract', id: id, day: day, check_in: r.check_in};
            loadHabits();
        };
        habitShow(id, value);
        if (existing)
            send('habit-correct', Object.assign({check_in: existing}, habitCheckValue(h, value)),
                 (r, ok) => ok ? done(r) : failed(), true);
        else
            send('habit-check-in', Object.assign({id: id, day: day}, habitCheckValue(h, value)),
                 (r, ok) => ok ? done(r) : failed(), true);
    }
    function habitRetract(u) {
        habitShow(u.id, null);
        send('habit-retract', {check_in: u.check_in}, (r, ok) => {
            if (!ok) message('Could not take the check-in back', true);
            loadHabits();
        }, true);
    }
    // Brings back the day's check-in a take-back hid (an untick or an undone
    // first press), whole, as habit-list names it.
    function habitRestore(id) {
        var h = habit(id);
        if (!h || !h.restorable || habitList.day !== habitShownDay) return;
        habitUndo = null;
        send('habit-restore', {retraction: h.restorable}, (r, ok) => {
            if (!ok) message('Could not restore the check-in', true);
            loadHabits();
        }, true);
    }
    // The control moves at once; the engine's answer then replaces the map.
    function habitShow(id, value) {
        var next = JSON.parse(JSON.stringify(habitMap));
        var row = next.habits.find(h => h.id === id);
        if (!row || !row.days.length) return;
        row.days[row.days.length - 1].value = value;
        habitMap = next;
    }
    function habitOrder(bundle, order, undoing) {
        var previous = (habitList.bundles[bundle] || []).slice();
        var next = JSON.parse(JSON.stringify(habitList));
        next.bundles[bundle] = order;
        habitList = next;
        send('habit-bundle-order', {bundle: bundle, order: order, day: today}, (r, ok) => {
            if (!ok) { message('Could not save the order', true); loadHabits(); return; }
            if (!undoing) habitUndo = {kind: 'order', bundle: bundle, order: previous};
        }, true);
    }
    function habitUndoLast() {
        var u = habitUndo;
        if (!u) return;
        habitUndo = null;
        if (u.kind === 'order') habitOrder(u.bundle, u.order, true);
        else if (u.day !== habitShownDay) return;
        else if (u.kind === 'retract') habitRetract(u);
        else habitCheckIn(u.id, u.value, true);
    }
    // spec: {id?, title, direction, baseline, bundle}; a changed bundle joins last.
    function habitSave(spec, cb) {
        var args = {title: spec.title, direction: spec.direction, baseline: spec.baseline, day: today};
        if (spec.id) args.id = spec.id;
        if (spec.bundle) args.bundle = spec.bundle;
        send('habit-define', args, (r, ok) => {
            if (!ok) message('Could not save the habit', true);
            else loadHabits();
            if (cb) cb(ok);
        }, true);
    }
    function habitMove(id, bundle) {
        var h = habit(id);
        if (h) habitSave({id: id, title: h.title, direction: h.direction, baseline: h.baseline, bundle: bundle});
    }
    function habitMarkPrivate(id) {
        habitSetPrivate(id, true);
    }
    function habitSetPrivate(id, on) {
        send(on ? 'ledger-mark-private' : 'ledger-unmark-private', {scope: 'item', key: id}, (r, ok) => {
            if (!ok) message('Could not save the private mark', true);
            loadHabits();
        }, true);
    }
    // A habit's place in the plan: its category, goal and ordered tasks, read
    // through habit-actions for the shown day. The habit stays a definition;
    // only its tasks' actual events reach the calendar.
    property var habitPlan: ({id:'', day:'', hierarchy:null, hierarchy_status:'unlinked', actions:[]})
    property bool habitPlanLoaded: false
    function loadHabitPlan(id) {
        if (!id) { habitPlan = {id:'', day:'', hierarchy:null, hierarchy_status:'unlinked', actions:[]}; habitPlanLoaded = false; return; }
        if (habitPlan.id !== id) habitPlanLoaded = false;
        var day = habitShownDay;
        send('habit-actions', {id: id, day: day, timezone: prefs.timezone, hide_all: habitHideAll}, (r, ok) => {
            if (!ok) { if (habitPlan.id !== id) habitPlanLoaded = false; return; }
            if (r.id === id && r.day === habitShownDay) { habitPlan = r; habitPlanLoaded = true; }
        }, true);
    }
    // links: {category_id, goal_id, task_ids}; a goal brings its own category.
    function habitLink(id, links) {
        send('habit-link', {id: id, category_id: links.category_id || null, goal_id: links.goal_id || null,
                            task_ids: links.task_ids || [], day: today}, (r, ok, error) => {
            if (!ok) message('Could not link the habit · ' + (error || ''), true);
            loadHabits();
            loadHabitPlan(id);
            refreshCalendar();
        }, true);
    }
    // Completes or reopens the one task occurrence behind an action; the
    // habit's own check-in is left as it is.
    function habitActionSet(id, action, done) {
        send('habit-action-set', {id: id, task_id: action.id, day: habitPlan.day || habitShownDay, timezone: prefs.timezone,
                                  occurrence: action.occurrence || null, occurrence_date: action.occurrence ? action.occurrence_date : null,
                                  hash: action.hash, done: !!done}, (r, ok) => {
            if (!ok) message('Could not save the task', true);
            else if (r.id === habitPlan.id) habitPlan = r;
            send('state', {}, (s, fresh) => { if (fresh) install(s); }, true);
            if (!ok) loadHabitPlan(id);
            refreshCalendar();
        }, true);
    }
    // The task opens in the planner's own editor, where it is scheduled.
    function openHabitTask(taskId) {
        if (!get(taskId)) return;
        navigate('Tasks', 'Tasks');
        select(taskId);
    }
    function openHabitForm(id) {
        if (page !== 'Hearth' || view !== 'Habits') navigate('Hearth', 'Habits');
        habitFormId = id || '';
        habitFormOpen = true;
        habitFormSerial++;
    }

    // Kitchen and Finance (Hearth): every list and figure below is read from
    // the engine's kitchen-* and finance-* calls and its week question;
    // nothing is counted or added up here.
    property var kitchenStock: []
    property var kitchenGoesOff: []
    property var kitchenEquipment: []
    property var kitchenProfile: []
    property bool kitchenLoaded: false
    readonly property int kitchenSpan: [3, 7, 14, 30].indexOf(prefs.kitchenSpan) >= 0 ? prefs.kitchenSpan : 3
    readonly property string kitchenTab: ['Stock', 'Equipment', 'Profile'].indexOf(prefs.kitchenTab) >= 0 ? prefs.kitchenTab : 'Stock'
    property var financeWeeks: []
    property bool financeLoaded: false
    // The one right pane of Kitchen and Finance: {kind, item?}. kind is one of
    // lot, newLot, equipment, newEquipment, constraint, newConstraint, plan,
    // import, bank; '' when closed.
    property var hearthPane: ({kind: ''})
    property int hearthPaneSerial: 0
    // The chosen statement file: handed to the engine, never shown.
    property string statementFile: ''
    property int statementPickSerial: 0
    property bool ledgerReadying: false

    onKitchenSpanChanged: if (page === 'Hearth' && view === 'Kitchen') loadGoesOff()

    // Sends a ledger call; a workspace without a ledger is readied first, as
    // Habits does, without a word.
    function ledgerSend(command, args, cb) {
        send(command, args, (r, ok, error) => {
            if (!ok && /no ledger yet/.test(error || '') && !ledgerReadying) {
                ledgerReadying = true;
                send('ledger-migrate', {}, (m, migrated) => {
                    ledgerReadying = false;
                    if (migrated) send(command, args, cb, true);
                    else cb(null, false, error);
                }, true);
                return;
            }
            cb(r, ok, error);
        }, true);
    }
    // The engine's reason in plain words, or '' when it carries an identifier,
    // a path or a kind of error a person cannot act on.
    function engineReason(error) {
        var text = String(error || '').replace(/^(ledger error|write conflict|invalid object|invalid date|codec error): /, '');
        if (!text || /^(I\/O error|invalid id|path denied|integrity|authorization|registry|recovery)/.test(text)) return '';
        if (/plan_|statement:|no current lot|no purchase line|[0-9a-f]{12,}|[A-Za-z]\/[A-Za-z]|^\//.test(text)) return '';
        if (/^Statement mapping:/.test(text)) return '';
        return text.charAt(0).toUpperCase() + text.slice(1);
    }
    function failNotice(words, error) {
        var reason = engineReason(error);
        message(reason ? words + ' · ' + reason : words, true);
    }
    function hearthReload() {
        if (page !== 'Hearth') return;
        if (view === 'Kitchen') loadKitchen();
        else if (view === 'Finance') loadFinance();
        else if (view === 'Now') loadNow();
    }
    function loadKitchen() {
        var failed = false;
        var fail = () => { if (!failed) { failed = true; message('Kitchen is not available right now', true); } };
        ledgerSend('kitchen-stock-list', {}, (r, ok) => {
            if (!ok) { fail(); return; }
            kitchenStock = r.lots || [];
            kitchenLoaded = true;
            loadGoesOff();
            send('kitchen-equipment-list', {}, (e, eok) => { if (eok) kitchenEquipment = e.equipment || []; else fail(); }, true);
            send('kitchen-profile-list', {}, (p, pok) => { if (pok) kitchenProfile = p.profile || []; else fail(); }, true);
        });
    }
    function loadGoesOff() {
        var span = kitchenSpan;
        send('kitchen-goes-off', {days: span, today: today}, (r, ok) => {
            if (!ok) { message('Kitchen is not available right now', true); return; }
            if (r.days !== kitchenSpan) return;
            var c = r.columns || [];
            kitchenGoesOff = (r.rows || []).map(row => {
                var o = {};
                c.forEach((name, i) => o[name] = row[i]);
                return o;
            });
        }, true);
    }
    function setKitchenSpan(days) { setting('kitchenSpan', days); }
    // 13.11 3a: a batch plan in progress, kept for the window's session as
    // unsaved edits are kept. Writing or discarding the plan clears it.
    property var planDraft: null
    function setKitchenTab(tab) {
        setting('kitchenTab', tab);
        if (tab === 'Stock' && planDraft) openHearthPane('plan');
        else hearthPane = {kind: ''};
    }
    function lot(id) { return kitchenStock.find(l => l.id === id) || null; }
    function openHearthPane(kind, item) {
        if (page !== 'Hearth' || ['lot', 'newLot', 'equipment', 'newEquipment', 'constraint', 'newConstraint', 'plan'].indexOf(kind) >= 0 && view !== 'Kitchen')
            navigate('Hearth', ['import', 'bank', 'lines'].indexOf(kind) >= 0 ? 'Finance' : 'Kitchen');
        habitFormOpen = false;
        hearthPane = {kind: kind, item: item || null};
        hearthPaneSerial++;
    }
    function closeHearthPane() { if (hearthPane.kind === 'plan') planDraft = null; hearthPane = {kind: ''}; }
    // The add button of the tab shown, as Ctrl+N and the palette run it.
    function kitchenAdd() {
        var tab = kitchenTab;
        openHearthPane(tab === 'Equipment' ? 'newEquipment' : tab === 'Profile' ? 'newConstraint' : 'newLot');
    }
    function kitchenAddLot(spec, cb) {
        var args = {subject: spec.subject, quantity: spec.quantity, unit: spec.unit, source: 'typed', day: today};
        if (spec.expires_on) args.expires_on = spec.expires_on;
        if (spec.location) args.location = spec.location;
        ledgerSend('kitchen-stock-add', args, (r, ok, error) => {
            if (!ok) failNotice('Could not save the lot', error);
            loadKitchen();
            cb(ok);
        });
    }
    // changes: {used?, quantity?, expires_on?}, written one at a time on the
    // lot each write leaves current: the count alone when both used and
    // quantity are given, otherwise Used first, then Goes off.
    function kitchenChangeLot(id, changes, cb) {
        var steps = [];
        if (changes.quantity !== undefined) steps.push(['kitchen-stock-adjust', {quantity: changes.quantity}]);
        else if (changes.used !== undefined) steps.push(['kitchen-stock-use', {amount: changes.used}]);
        if (changes.expires_on) steps.push(['kitchen-stock-expiry', {expires_on: changes.expires_on}]);
        if (changes.location) steps.push(['kitchen-lot-move', {location: changes.location}]);
        var run = (current, i) => {
            if (i >= steps.length) { loadKitchen(); cb(true); return; }
            send(steps[i][0], Object.assign({lot: current, day: today}, steps[i][1]), (r, ok, error) => {
                if (!ok) { failNotice('Could not save the lot', error); loadKitchen(); cb(false); return; }
                run(r.id, i + 1);
            }, true);
        };
        run(id, 0);
    }
    function kitchenUsedUp(id) {
        send('kitchen-stock-adjust', {lot: id, quantity: 0, day: today}, (r, ok, error) => {
            if (!ok) failNotice('Could not save the lot', error);
            loadKitchen();
        }, true);
    }
    function kitchenSetEquipment(name, count, capacity, cb) {
        var args = {name: name, count: count, day: today};
        if (capacity) args.capacity = capacity;
        ledgerSend('kitchen-equipment-set', args, (r, ok, error) => {
            if (!ok) failNotice('Could not save the equipment', error);
            loadKitchen();
            if (cb) cb(ok);
        });
    }
    // entries: [{subject, quantity?, unit?, value?}], one write each.
    function kitchenSetProfile(entries, cb) {
        var run = i => {
            if (i >= entries.length) { loadKitchen(); cb(true); return; }
            var e = entries[i], args = {subject: e.subject, day: today};
            if (e.value !== undefined) args.value = e.value;
            else { args.quantity = e.quantity; if (e.unit) args.unit = e.unit; }
            ledgerSend('kitchen-profile-set', args, (r, ok, error) => {
                if (!ok) { failNotice('Could not save the constraint', error); loadKitchen(); cb(false); return; }
                run(i + 1);
            });
        };
        run(0);
    }
    // dishes: [{name, portions, phase, equipment: [names], uses: [{lot, qty}]}].
    function kitchenWritePlan(day, dishes, cb) {
        ledgerSend('kitchen-batch-plan', {day: day, horizon_days: kitchenSpan, dishes: dishes}, (r, ok, error) => {
            if (!ok) {
                var text = String(error || '');
                if (/is already written/.test(text)) message('A plan for this day is already written', true);
                else if (/no current lot/.test(text)) message('Could not write the plan · an ingredient is no longer in stock', true);
                else failNotice('Could not write the plan', error);
                cb(null);
                return;
            }
            loadKitchen();
            cb(r);
        });
    }

    function loadFinance() {
        ledgerSend('ledger-question', {name: 'spend-by-category-weeks'}, (r, ok) => {
            if (!ok) { message('Finance is not available right now', true); return; }
            var c = r.columns || [], weeks = {};
            (r.rows || []).forEach(row => {
                var o = {};
                c.forEach((name, i) => o[name] = row[i]);
                (weeks[o.week] = weeks[o.week] || []).push(o);
            });
            financeWeeks = Object.keys(weeks).sort().reverse().map(w => ({week: w, rows: weeks[w]}));
            financeLoaded = true;
        });
    }
    // A week's lines of one category (finance-lines, T027), and a line's
    // category changed by superseding it (finance-recategorise).
    function financeLines(week, category, cb) {
        send('finance-lines', {week: week, category: category}, (r, ok) => {
            if (!ok) { message('Finance is not available right now', true); cb([]); return; }
            cb((r.lines || []).filter(l => l.week === week || !l.week));
        }, true);
    }
    function financeRecategorise(line, category, cb) {
        send('finance-recategorise', {line: line, category: category}, (r, ok, error) => {
            if (!ok) failNotice('Could not save the category', error);
            loadFinance();
            cb(ok);
        }, true);
    }
    // The plan written for a day (kitchen-plans, T027), or null.
    function kitchenPlanFor(day, cb) {
        send('kitchen-plans', {day: day}, (r, ok) => {
            var plans = ok ? (r.plans || []).filter(p => p.day === day) : [];
            cb(plans.length ? plans[0] : null);
        }, true);
    }
    // T028: a profile entry removed, a lot moved, a dish cooked, the banks set
    // up and a chosen statement's first line.
    function kitchenRemoveProfile(subject) {
        send('kitchen-profile-remove', {subject: subject, day: today}, (r, ok, error) => {
            if (!ok) failNotice('Could not save the constraint', error);
            loadKitchen();
        }, true);
    }
    function kitchenCook(planId, dishId, cb) {
        send('kitchen-cook', {plan: planId, dish: dishId, day: today}, (r, ok, error) => {
            if (!ok) failNotice('Could not save the lot', error);
            loadKitchen();
            cb(ok);
        }, true);
    }
    property var financeBanks: []
    function loadBanks(cb) {
        send('finance-banks', {}, (r, ok) => { financeBanks = ok ? r.banks || [] : []; if (cb) cb(); }, true);
    }
    function statementHeader(cb) {
        if (!statementFile) { cb([]); return; }
        send('finance-statement-header', {path: statementFile}, (r, ok) => cb(ok ? r.columns || [] : []), true);
    }
    function chooseStatement() {
        if (page !== 'Hearth' || view !== 'Finance') navigate('Hearth', 'Finance');
        statementPickSerial++;
    }
    function statementChosen(path) {
        statementFile = path;
        openHearthPane('import', {bank: ''});
    }
    // cb({ok, setup}): setup when the bank has no mapping yet.
    function financeImport(bank, cb) {
        ledgerSend('finance-statement-import', {bank: bank, path: statementFile}, (r, ok, error) => {
            if (!ok) {
                if (/no statement mapping/.test(error || '')) { cb({ok: false, setup: true}); return; }
                failNotice('Could not import the statement', error);
                cb({ok: false});
                return;
            }
            var plural = (n, one, many) => n + ' ' + (n === 1 ? one : many);
            var period = String(r.period || '').split('..').map(d => d ? longDate(d) : '');
            message(plural(r.added, 'line added', 'lines added') + ' · ' + r.duplicates + ' already in · '
                    + plural(r.credits, 'payment in', 'payments in') + ', left out\n'
                    + bank + (period[0] ? ', ' + (period[1] && period[1] !== period[0] ? period[0] + ' to ' + period[1] : period[0]) : ''));
            statementFile = '';
            loadFinance();
            cb({ok: true});
        });
    }
    function financeSaveBank(mapping, cb) {
        ledgerSend('finance-statement-mapping', mapping, (r, ok, error) => {
            if (!ok) failNotice('Could not save the bank', error);
            cb(ok);
        });
    }
    // The ledger's category keys as a person reads them (sheet 13.7).
    readonly property var categoryWords: [
        {key: 'food:groceries', word: 'Groceries'}, {key: 'food:eating_out', word: 'Eating out'},
        {key: 'food:home', word: 'Home cooking'}, {key: 'household:utilities', word: 'Utilities'},
        {key: 'household:other', word: 'Household'}, {key: 'transport', word: 'Transport'},
        {key: 'health', word: 'Health'}, {key: 'other', word: 'Other'},
        {key: 'uncategorised', word: 'Not yet categorised'}
    ]
    function categoryWord(key) {
        var known = categoryWords.find(c => c.key === key);
        if (known) return known.word;
        var text = String(key || '').replace(/[^A-Za-z0-9]+/g, ' ').trim();
        return text.charAt(0).toUpperCase() + text.slice(1);
    }
    // GBP, EUR and USD by their symbol; any other currency by its code after.
    function moneyText(amount, currency) {
        var symbols = {GBP: '£', EUR: '€', USD: '$'};
        var text = String(amount);
        if (symbols[currency]) return text.charAt(0) === '-' ? '-' + symbols[currency] + text.slice(1) : symbols[currency] + text;
        return text + ' ' + currency;
    }
    // "6 October"; the year only when not this year.
    function longDate(iso) {
        var p = String(iso || '').split('-');
        if (p.length !== 3) return '';
        var d = new Date(Number(p[0]), Number(p[1]) - 1, Number(p[2]));
        return d.toLocaleDateString(Qt.locale('en_GB'), p[0] === today.substring(0, 4) ? 'd MMMM' : 'd MMMM yyyy');
    }
    // A day as the sheet writes it in running text: "Monday 6 October", the year only when not this year.
    function dayDate(iso) {
        var p = String(iso || '').split('-');
        if (p.length !== 3) return '';
        var d = new Date(Number(p[0]), Number(p[1]) - 1, Number(p[2]));
        return d.toLocaleDateString(Qt.locale('en_GB'), p[0] === today.substring(0, 4) ? 'dddd d MMMM' : 'dddd d MMMM yyyy');
    }
    // A quantity as the engine gave it, without trailing zeros.
    function amountText(value, unit) {
        if (value === null || value === undefined) return '';
        var n = Math.round(Number(value) * 1000) / 1000;
        return String(n) + (unit ? ' ' + unit : '');
    }
    function routeState() {
        return {selectedId:selectedId, activeKey:activeKey, detailsOpen:hasRouteDetails,
                detailFull:detailFull && hasRouteDetails, detailRoute:detailRoute,
                compactPane:compactPane, branches:branches.slice(), search:search,
                priority:priority, status:status, board:board, goalList:goalList,
                showArchived:showArchived, selectedDate:selectedDate, calendarMode:calendarMode};
    }
    function storedRouteContexts() {
        var contexts = Object.assign({}, routeContexts);
        contexts[routeKey] = routeState();
        return contexts;
    }
    function closeDetails() {
        detailsOpen = false;
        detailFull = false;
        compactPane = 'work';
        remember();
    }
    function navigateView(v) {
        var target = Routes.normalize('Action Center', v || 'Timeline');
        navigate(target.page, target.view);
    }
    function navigate(p, v) {
        var target = Routes.normalize(p, v || routeViews[p]);
        var key = target.page + '/' + target.view;
        if (key === routeKey) return;
        routeLeaving();
        routeContexts = storedRouteContexts();
        // Calendar horizons share scope; object selection remains route-owned.
        var scopeContext = routeContexts[key];
        if (target.page === 'Dashboard') {
            var scopeKey = page === 'Dashboard' ? routeKey : 'Dashboard/' + (routeViews.Dashboard || target.view);
            scopeContext = routeContexts[scopeKey] || scopeContext;
        }
        var views = Object.assign({}, routeViews);
        views[page] = view;
        views[target.page] = target.view;
        routeViews = views;
        restoringRoute = true;
        var context = routeContexts[key];
        page = target.page;
        view = target.view;
        selectedId = context ? context.selectedId || '' : '';
        activeKey = context ? context.activeKey || '' : '';
        detailsOpen = context ? !!context.detailsOpen : false;
        detailRoute = context && context.detailsOpen ? key : '';
        detailFull = context ? !!context.detailFull : false;
        compactPane = context && context.detailsOpen ? 'detail' : 'work';
        if (scopeContext) {
            branches = (scopeContext.branches || []).slice();
            search = scopeContext.search || '';
            priority = scopeContext.priority || 'all';
            status = scopeContext.status || 'all';
            showArchived = !!scopeContext.showArchived;
            if (target.page === 'Dashboard') selectedDate = scopeContext.selectedDate || selectedDate;
        }
        if (context) {
            board = !!context.board;
            goalList = !!context.goalList;
            if (target.page === 'Dashboard') calendarMode = context.calendarMode || calendarMode;
        }
        if (target.page === 'Dashboard' && ['Day', 'Week', 'Month', 'Year'].indexOf(target.view) >= 0)
            calendarMode = target.view;
        restoringRoute = false;
        remember();
    }
    function placeRecord(r, target, after) {
        if (r.id === target.id || r.kind !== target.kind)
            return;
        if (Object.values(drafts).some(d => d.id === r.id && d.dirty)) {
            message('Save or discard this card’s draft before moving it.', true);
            return;
        }
        send('reorder', {
            id: r.id,
            hash: r.hash,
            target: target.id,
            after: !!after
        }, (v, ok) => {
            if (ok)
                message('Moved ' + r.title);
        });
    }
    // The private marks are the engine's: they arrive filtered while private
    // mode is on, so the window never writes them back.
    function preferenceState() {
        var kept = Object.assign({}, prefs);
        delete kept.privateIds;
        delete kept.privateModules;
        return Object.assign(kept, {
            page: page,
            view: view,
            calendarMode: calendarMode,
            selectedDate: selectedDate,
            selectedId: selectedId,
            activeKey: activeKey,
            drafts: drafts,
            detailsOpen: detailsOpen,
            detailRoute: detailRoute,
            detailFull: detailFull,
            routeContexts: storedRouteContexts(),
            routeViews: routeViews,
            consoleOpen: consoleOpen,
            slotModule: slotModule,
            navCollapsed: navCollapsed,
            branches: branches,
            search: search,
            priority: priority,
            status: status,
            board: board,
            goalList: goalList,
            showCompleted: showCompleted,
            showArchived: showArchived
            ,developerSection: developerSection, developerModule: developerModule, strategyTab: strategyTab, historySelectedId: historySelectedId, proposalDraft: proposalDraft
        });
    }
    function refreshCalendar() {
        if (!available || !restored)
            return;
        calendarTimer.restart();
        refreshProposalCalendar();
    }
    function refreshProposalCalendar() {
        if (!available || !restored) return;
        var year=Number(selectedDate.substring(0,4));
        send('proposal-calendar',{from:(year-1)+'-12-25',through:(year+1)+'-01-07',timezone:prefs.timezone},(r,ok)=>{if(ok)proposalEvents=r || [];},true);
    }
    function reloadDraft() {
        if (!activeDraft)
            return;
        var d = JSON.parse(JSON.stringify(activeDraft));
        var id = d.id, event = null;
        if (d.occurrence) {
            event = events.find(e => e.id === id && e.occurrence === d.occurrence) || null;
            if (!event) {
                message('Occurrence reload unavailable. Your unsaved draft is preserved.', true);
                return;
            }
        }
        var map = Object.assign({}, drafts);
        delete map[activeKey];
        drafts = map;
        if (d.occurrence)
            select(id, event);
        else if (id)
            select(id);
        else {
            activeKey = '';
            detailsOpen = false;
        }
        remember();
    }
    function remember() {
        if (restored) {
            prefsDirty = true;
            prefTimer.restart();
        }
    }
    function saveDraft() {
        if (!activeDraft || saving)
            return;
        var key = activeKey, d = JSON.parse(JSON.stringify(activeDraft));
        saving = true;
        var command = d.occurrence && d.occurrenceOnly ? 'occurrence' : 'save';
        var priorOccurrence = command === 'occurrence'
            ? events.find(e => e.id === d.id && e.occurrence === d.occurrence) : null;
        var priorStatus = priorOccurrence ? priorOccurrence.status : '';
        var args = command === 'occurrence' ? {
            id: d.id,
            hash: d.hash,
            occurrence: d.occurrence,
            occurrence_date: d.occurrence_date,
            date: d.scheduled_for,
            title: d.title,
            status: d.status,
            schedule: d.schedule
        } : d;
        send(command, args, (r, ok) => {
            saving = false;
            if (!ok)
                return;
            var map = Object.assign({}, drafts);
            // Preserve text typed while the save was in flight.
            if (JSON.stringify(map[key]) === JSON.stringify(d)) {
                delete map[key];
                if (activeKey === key) {
                    selectedId = r.id;
                    if (d.occurrence && d.occurrenceOnly) {
                        map[key] = Object.assign({}, d, r, {
                            occurrence: d.occurrence,
                            occurrence_date: d.occurrence_date,
                            occurrence_sequence: d.occurrence_sequence,
                            occurrenceOnly: true,
                            scheduled_for: d.scheduled_for,
                            schedule: d.schedule,
                            title: d.title,
                            status: d.status,
                            dirty: false
                        });
                    } else {
                        activeKey = r.id;
                        map[r.id] = Object.assign({}, r, {
                            dirty: false
                        });
                    }
                }
            } else if (map[key]) {
                map[key] = d.occurrence && d.occurrenceOnly ? Object.assign({}, map[key], {
                    id: r.id,
                    hash: r.hash,
                    revision: r.revision
                }) : Object.assign({}, map[key], {
                    id: r.id,
                    hash: r.hash
                });
            }
            drafts = map;
            if (command === 'occurrence' && d.status === 'completed' && priorStatus && priorStatus !== 'completed') {
                var completionNow = Date.now();
                setting('lastTaskCompletion', {entries:[{id:d.id,previous:priorStatus,
                    afterHash:r.hash,occurrence:d.occurrence,occurrence_date:d.occurrence_date}],
                    graceUntil:completionNow+4000,undoUntil:completionNow+30000});
            }
            remember();
            message('Saved · ' + r.title, false);
        });
    }
    function saveView(name) {
        if (!name.trim())
            return;
        var list = (prefs.savedViews || []).filter(v => v.name !== name);
        list.push({
            name: name,
            page: page,
            view: view,
            branches: branches,
            priority: priority,
            status: status,
            search: search,
            board: board,
            calendarMode: calendarMode
        });
        setting('savedViews', list);
        message('Saved view · ' + name);
    }
    function deleteView(name) {
        var list = (prefs.savedViews || []).filter(v => v.name !== name);
        setting('savedViews', list);
        message('Deleted view · ' + name);
    }
    function select(id, event) {
        var r = get(id);
        if (!r)
            return;
        selectedId = id;
        activeKey = event && event.occurrence ? id + ':' + event.occurrence : id;
        if (event && event.occurrence && (!drafts[activeKey] || !drafts[activeKey].dirty)) {
            var occurrenceDraft = JSON.parse(JSON.stringify(event));
            occurrenceDraft.scheduled_for = event.date;
            occurrenceDraft.dirty = false;
            occurrenceDraft.occurrenceOnly = true;
            var occurrenceMap = Object.assign({}, drafts);
            occurrenceMap[activeKey] = occurrenceDraft;
            drafts = occurrenceMap;
        } else if (!drafts[activeKey]) {
            var d = JSON.parse(JSON.stringify(r));
            d.dirty = false;
            if (event && event.occurrence) {
                d.occurrence = event.occurrence;
                d.occurrence_date = event.occurrence_date;
                d.occurrence_sequence = event.occurrence_sequence;
                d.scheduled_for = event.date;
                d.schedule = event.schedule;
                d.title = event.title;
                d.occurrenceOnly = true;
            }
            var map = Object.assign({}, drafts);
            map[activeKey] = d;
            drafts = map;
        }
        detailsOpen = true;
        detailRoute = routeKey;
        compactPane = 'detail';
        remember();
    }
    property var collectionStatus: ({})
    property bool collectionPolling:false
    property Timer collectionTimer: Timer {
        interval:1500;repeat:true;running:b.available && b.restored && !b.closing && Quickshell.env('SOLITON_COLLECTION_AUTOSTART') === '1'
        onTriggered: {
            if(b.collectionPolling)return;b.collectionPolling=true;
            b.send('collection-tick',{},(r,ok,e)=>{b.collectionPolling=false;if(ok)b.collectionStatus=r;else b.collectionStatus={archive_import_error:e};},true);
        }
    }
    property var activeHistoryJob: null
    property var historyRequests: []
    property bool captureBootstrapSent: false
    function bootstrapCapture() {
        if (captureBootstrapSent || prefs.automaticCapture !== true || Quickshell.env('SOLITON_AUTO_CAPTURE') === '0')
            return;
        captureBootstrapSent = true;
        send('history-capture-bootstrap', {}, (value, started, error) => {
            if (!started && error)
                console.warn('History capture bootstrap unavailable:', error);
        }, true);
    }
    property bool historyStarting: false
    function historyCallback(job,result,ok,error) {
        if(job.cb) { try { job.cb(result,ok,error); } catch(e) { console.warn('Closed history view:',e); } }
        // A caller with a callback owns its local error lifetime. Do not also
        // leak its error into the global shell after the route has changed.
        if(!ok && !job.quiet && !job.cb)message(error || 'History operation failed',true);
    }
    function pumpHistory() {
        if(!available || historyStarting || activeHistoryJob || !historyRequests.length)return;
        var queue=historyRequests.slice(),job=queue.shift();historyRequests=queue;historyStarting=true;
        send('history-job-start',{command:job.command,args:job.args},(r,ok,e)=>{
            historyStarting=false;
            if(ok && r.state==='busy') {historyRequests=[job].concat(historyRequests);return;}
            if(ok)activeHistoryJob={id:r.id,cb:job.cb,quiet:job.quiet};
            else historyCallback(job,null,false,e);
        },true);
    }
    property Timer historyQueueTimer: Timer {
        interval:250;repeat:true;running:b.available && b.historyRequests.length>0
        onTriggered:b.pumpHistory()
    }
    property bool historyPolling: false
    property Timer historyJobTimer: Timer {
        interval:250;repeat:true;running:b.available && !!b.activeHistoryJob
        onTriggered: {
            if(b.historyPolling)return;
            b.historyPolling=true;
            b.send('history-job-status',{id:b.activeHistoryJob.id},(r,ok,e)=>{
                b.historyPolling=false;
                if(!b.activeHistoryJob)return;
                if(ok && r.state==='running')return;
                var job=b.activeHistoryJob;b.activeHistoryJob=null;
                b.historyCallback(job,ok?r.result:null,ok && !!r.ok,ok?r.error:e);
                b.pumpHistory();
            },true);
        }
    }
    function send(command, args, cb, quietError) {
        if(['history-capture-bootstrap','history-capture-list','history-capture-add','history-capture-remove','history-capture-sync','history-import','history-import-batch','history-discover','history-opencode-sessions','history-from-inbox','history-list','history-detail','history-word-timings','history-revisions','history-correct','history-label','history-state'].includes(command)) {
            var job={command:command,args:args || {},cb:cb,quiet:quietError};
            if(!available || closing){historyCallback(job,null,false,'Local service is unavailable. Retry when connected.');return;}
            if(historyRequests.length>=32){historyCallback(job,null,false,'History queue is full. Wait for the current operations to finish.');return;}
            historyRequests=historyRequests.concat([job]);pumpHistory();return;
        }
        if (!available) {
            message('Local service is unavailable. Your text remains in this window.', true);
            if (cb)
                cb(null, false);
            return;
        }
        var id = ++sequence;
        if (quietError) quietErrors[id] = true;
        if (cb)
            callbacks[id] = cb;
        bridge.write(JSON.stringify({
            id: id,
            command: command,
            args: args || {}
        }) + '\n');
    }
    function setting(key, value) {
        if (key === 'showCompleted') {
            showCompleted = !!value;
            return;
        }
        var p = Object.assign({}, prefs);
        p[key] = value;
        prefs = p;
        remember();
        if (key === 'timezone') refreshCalendar();
        if (key === 'automaticCapture') bootstrapCapture();
    }
    function shortPath(path) {
        return path.split('/').slice(-3).join('/');
    }
    function shutdown() {
        if (closing)
            return;
        closing = true;
        closeWait.start();
    }
    function disconnectService() {
        if (bridge.running)
            bridge.running = false;
    }
    function reconnect() {
        if (!bridge.running)
            bridge.running = true;
    }
    function toggleBranch(id) {
        branches = branches.indexOf(id) >= 0 ? branches.filter(v => v !== id) : branches.concat([id]);
        remember();
    }

    onBoardChanged: remember()
    onCalendarModeChanged: remember()
    // One module in the slot: a flag set elsewhere (palette, shortcut, a view) goes through openTool.
    onConsoleOpenChanged: {
        if (slotSyncing) return;
        if (consoleOpen !== isConsoleModule(slotModule)) openTool(consoleOpen ? 'console' : '');
    }
    onDetailsOpenChanged: remember()
    onGoalListChanged: remember()
    onNavCollapsedChanged: remember()
    onPageChanged: {
        remember();
        if (page === 'Hearth') hearthOpened();
        else { habitFormOpen = false; hearthPane = {kind: ''}; }
    }
    onPriorityChanged: remember()
    onSearchChanged: remember()
    onSelectedDateChanged: {
        remember();
        refreshCalendar();
    }
    onShowArchivedChanged: remember()
    onShowCompletedChanged: remember()
    onStatusChanged: remember()
    onViewChanged: {
        remember();
        if (page === 'Hearth') hearthOpened();
    }
    // The Hearth module on screen; it changes only while Hearth is shown.
    property string hearthView: 'Habits'
    // The one list of Hearth's modules: its tab strip, the side panel and the
    // loader read it, in this order.
    readonly property var hearthViews: ['Habits', 'Kitchen', 'Finance', 'Now']
    // Now: the engine's bounded list for the day, read whole from now-view;
    // nothing is ranked, counted or cut here. nowAt names another day or time
    // for the engine ('' is today and the clock).
    property var nowList: ({day: '', items: [], left_out: 0})
    property bool nowLoaded: false
    property var nowAt: ({day: '', time: ''})
    property string nowLotRequest: ''
    function loadNow() {
        var args = {};
        if (nowAt.day) args.day = nowAt.day;
        if (nowAt.time) args.time = nowAt.time;
        ledgerSend('now-view', args, (r, ok, error) => {
            if (!ok) { failNotice('Now is not available right now', error); return; }
            nowList = r;
            nowLoaded = true;
        });
        // A habit's check-in from Now goes through the day's habit list.
        if (!habitsLoaded && !habitsReadying) loadHabits();
    }
    // His objective is preferences: each change is written, then read back.
    function nowSetting(key, value) {
        setting(key, value);
        flushPreferences(() => loadNow());
    }
    // A lot opens in Kitchen's Stock with its pane once the stock is read.
    function openNowLot(id) {
        nowLotRequest = id;
        navigate('Hearth', 'Kitchen');
        setKitchenTab('Stock');
        if (lot(id)) { nowLotRequest = ''; openHearthPane('lot', lot(id)); }
    }
    onKitchenStockChanged: if (nowLotRequest && lot(nowLotRequest)) { var l = lot(nowLotRequest); nowLotRequest = ''; openHearthPane('lot', l); }
    function hearthOpened() {
        hearthView = view;
        if (view !== 'Habits') habitFormOpen = false;
        if (view === 'Habits' || view === 'Now' || ['import', 'bank', 'lines'].indexOf(hearthPane.kind) >= 0 !== (view === 'Finance')) hearthPane = {kind: ''};
        if (view === 'Habits') loadHabits();
        else hearthReload();
    }
}
