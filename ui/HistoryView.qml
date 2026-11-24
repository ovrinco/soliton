pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtMultimedia
import Quickshell
import Quickshell.Io
import Quickshell.Services.Pipewire
import "TranscriptDocument.js" as TranscriptDocument
import "transcript"
import "transcript/TranscriptTiming.js" as TranscriptTiming

Item {
    id: root
    objectName: "history-view"
    readonly property alias playbackOutput: audioPlayer.audioOutput
    readonly property int playbackPosition: audioPlayer.position
    readonly property int playbackDuration: audioPlayer.duration
    // The player keeps the last file's length after its source is cleared.
    readonly property int loadedDuration: audioPlaybackSource ? audioPlayer.duration : 0
    readonly property int playbackState: audioPlayer.playbackState

    signal proposalRequested(var record, var segment)
    signal recordKindResolved(string kind)
    property string requestedKind: 'chat'
    property var kindContexts: ({})
    property int restorePlaybackPosition: 0
    property bool switchingKind: false
    property int consumedNavigationSerial: 0
    property bool resolvingNavigationKind: false
    property string pendingResolvedKind: ''
    function hasExplicitNavigation() {
        return Backend.historyNavigation && Number(Backend.historyNavigation.serial) > consumedNavigationSerial;
    }
    Connections {
        target: Backend
        function onHistoryNavigationChanged() { if (root.contextReady && root.visible) root.syncFromBackend(); }
    }
    onRequestedKindChanged: if (contextReady) activateKind(requestedKind)

    // The page's four collections. Sources has no route of its own yet: it is
    // opened here, and leaving it follows the route again.
    readonly property var collections: [{kind:'chat', label:'Chats'}, {kind:'voice_note', label:'Voice notes'},
        {kind:'run', label:'Runs'}, {kind:'source', label:'Sources'}]
    function collectionLabel(value) {
        var found = collections.find(c => c.kind === value);
        return found ? found.label : 'Chats';
    }
    function openCollection(next) {
        if (next === 'source') { Backend.strategyTab = 'Sources'; activateKind('source'); return; }
        var tab = collectionLabel(next);
        Backend.strategyTab = tab;
        Backend.navigate('Strategy', tab);
        if (kind !== next) activateKind(next);
    }
    Connections {
        target: Backend
        function onStrategyTabChanged() {
            if (root.contextReady && root.kind === 'source' && Backend.strategyTab !== 'Sources') root.activateKind(root.requestedKind);
        }
    }

    function rememberKind() {
        if (!contextReady || switchingKind || resolvingNavigationKind) return;
        persistCorrectionDraft();
        var contexts = Object.assign({}, kindContexts);
        contexts[kind] = {query:query, offset:offset, items:items, total:total,
            selectedId:selectedId, selectedRecord:selectedRecord, revisions:revisions,
            selectedRevision:selectedRevision, detailFocused:detailFocused,
            position:audioPlayer.position, audioSource:audioPlaybackSource};
        kindContexts = contexts;
        var saved = Object.assign({}, Backend.prefs.recordViews || {});
        saved[kind] = {query:query, offset:offset, selectedId:selectedId, detailFocused:detailFocused};
        Backend.setting('recordViews', saved);
    }
    function activateKind(next, initial) {
        if (!initial && next === kind) return;
        if (!initial) rememberKind();
        switchingKind = true;
        stopAudio();
        editingSegment = false;
        sourceDialog.close(); recordActions.close();
        listRequest = nextRequest(); detailRequest = nextRequest(); revisionRequest = nextRequest();
        // Each view keeps its own list context; a pending explicit request
        // replaces only the selection, after that context is restored.
        var explicit = hasExplicitNavigation();
        resolvingNavigationKind = false;
        var c = kindContexts[next] || (Backend.prefs.recordViews || {})[next] || {};
        kind = next; query = c.query || ''; offset = Number(c.offset || 0);
        items = c.items || []; total = Number(c.total || 0);
        selectedId = c.selectedId || ''; selectedRecord = c.selectedRecord || null;
        revisions = c.revisions || []; selectedRevision = c.selectedRevision || '';
        detailFocused = !!c.detailFocused; selectedSegmentId = ''; passageTarget = null;
        errorText = ''; actionNotice = ''; detailBusy = false; listBusy = false;
        audioPlaybackSource = c.audioSource || ''; restorePlaybackPosition = Number(c.position || 0);
        switchingKind = false;
        if (visible && Backend.available) {
            loadList(false);
            if (explicit) syncFromBackend();
            else if (selectedId && !selectedRecord) selectRecord({id:selectedId},detailFocused);
            else if (selectedRecord) restoreCorrectionDraft();
        }
    }

    property var items: []
    property int total: 0
    property int offset: 0
    property int limit: 24
    property string query: ''
    property string recordState: 'active'
    property var lifecycleTarget: null
    function changeRecordState(state, confirmed) {
        var target = confirmed ? lifecycleTarget : selectedRecord;
        if (!target || !target.id || actionBusy) return;
        if (editingSegment) { errorText = 'Save or discard your transcript edit first.'; return; }
        var navigationAtRequest = Number((Backend.historyNavigation || {}).serial || 0);
        actionBusy = true;
        Backend.send('history-state', {id:target.id, expected_revision:target.revision, state:state, confirmed:!!confirmed}, (result, ok, error) => {
            actionBusy = false;
            if (!ok) { errorText = error; return; }
            if (selectedId === String(target.id)) {
                if (Number((Backend.historyNavigation || {}).serial || 0) > navigationAtRequest) {
                    // A newer explicit ID request wins over this older action reply.
                    selectRecord({id:target.id}, true);
                } else {
                    stopAudio(); selectedId = ''; selectedRecord = null;
                    selectedRevision = ''; revisions = []; detailFocused = false;
                    detailRequest = nextRequest(); detailBusy = false;
                    syncToBackend(); rememberKind();
                }
            }
            loadList(false);
        }, true);
    }
    property string kind: 'all'
    property string selectedId: ''
    property bool detailFocused: false
    property bool contextReady: false
    onQueryChanged: if (contextReady && !switchingKind) rememberKind()
    onKindChanged: if (contextReady && kind !== 'source') Backend.setting('historyKind', kind)
    onOffsetChanged: if (contextReady && !switchingKind) rememberKind()
    property var selectedRecord: null
    property var revisions: []
    property string selectedRevision: ''
    property string selectedSegmentId: ''
    property string correctionText: ''
    property string correctionReason: ''
    property var correctionEdits: []
    property string voiceDraftText: ''
    readonly property var voiceEdit: voiceDraftText ? (transcriptDocument.sources ? TranscriptTiming.paragraphEdit(transcriptDocument, voiceDraftText) : TranscriptDocument.edit(transcriptDocument, voiceDraftText)) : null
    property bool editingSegment: false
    readonly property string correctionDraftKey: 'historyCorrectionDraft'
    property bool listBusy: false
    property bool detailBusy: false
    property bool actionBusy: false
    property string errorText: ''
    property string actionNotice: ''
    property int requestSerial: 0
    property int listRequest: 0
    property int detailRequest: 0
    property int actionRequest: 0
    property int revisionRequest: 0
    property string importPath: Quickshell.env('HOME') + '/.local/share/voxtype'
    property string importProvider: 'voxtype'
    property string lastAudioError: ''
    // The engine's readable paragraphs; the segment document when it has none.
    readonly property var transcriptDocument: TranscriptTiming.paragraphDocument(segments, selectedRecord ? selectedRecord.paragraphs : null, TranscriptDocument.build(segments))
    readonly property bool isVoice: !!selectedRecord && selectedRecord.kind === 'voice_note' && kind === 'voice_note'
    // A run is read only: its lines are what the campaign reader found, never a transcript to edit.
    readonly property bool isRun: !!selectedRecord && selectedRecord.kind === 'run'
    readonly property var runLines: isRun ? segments.map(s => /^(Where|What|When|Result): ([\s\S]*)$/.exec(String(s.text || ''))).filter(m => !!m).map(m => ({label: m[1], text: m[2]})) : []
    property var passageTarget: null
    // Transcript playback: word timings from the engine, the speed as left, and
    // whether the transcript has unsaved changes (it is edited in place).
    property var wordTimings: null
    property bool wordTimingFailed: false
    property int wordTimingRequest: 0
    property bool hasPlayed: false
    readonly property bool correcting: !!voiceDraftText
    readonly property var placedTimings: wordTimings ? TranscriptTiming.place(transcriptDocument, wordTimings) : ({spans: [], anyEstimated: false})
    readonly property bool pitchKept: audioPlayer.pitchCompensationAvailability !== MediaPlayer.Unavailable
    readonly property real speedCeiling: pitchKept ? 2 : 1.5
    readonly property real playbackRate: Math.max(0.5, Math.min(speedCeiling, Number(Backend.prefs.historyPlaybackRate || 1)))
    readonly property bool voiceAudioReady: isVoice && !!selectedAudioPath() && !!selectedRecord.audio && selectedRecord.audio.state === 'retained'
    function setPlaybackRate(rate) { Backend.setting('historyPlaybackRate', Math.max(0.5, Math.min(speedCeiling, rate))); }
    function loadWordTimings(attempt) {
        var record = root.selectedRecord, serial = ++root.wordTimingRequest;
        attempt = attempt || 0;
        if (!attempt) { root.wordTimings = null; root.wordTimingFailed = false; }
        if (!record || record.kind !== 'voice_note' || !record.id) return;
        var id = String(record.id), revision = String(record.revision || '');
        Backend.send('history-word-timings', {id: id}, (value, ok) => {
            if (serial !== root.wordTimingRequest || !root.selectedRecord || String(root.selectedRecord.id) !== id) return;
            if (ok && value && String(value.revision || '') === revision) root.wordTimings = value;
            else if (!ok && attempt < 3) wordTimingRetry.restart();
            else root.wordTimingFailed = true;
        }, true);
    }
    Timer {
        // The history service answers nothing else while it imports; ask again shortly.
        id: wordTimingRetry
        interval: 1500
        property int attempt: 0
        onTriggered: root.loadWordTimings(++attempt)
    }
    property string timedRecordKey: ''
    onSelectedRecordChanged: {
        var key = selectedRecord ? String(selectedRecord.id || '') + '@' + String(selectedRecord.revision || '') : '';
        if (key === timedRecordKey) return;
        if (!selectedRecord || String(selectedRecord.id || '') !== timedRecordKey.split('@')[0]) hasPlayed = false;
        timedRecordKey = key;
        wordTimingRetry.stop(); wordTimingRetry.attempt = 0;
        loadWordTimings();
    }
    // A click in the transcript moves playback there; it starts nothing, since
    // the same click also places the caret for editing.
    function seekTo(milliseconds) {
        if (!root.voiceAudioReady) return;
        var target = Math.max(0, Math.round(milliseconds));
        if (root.audioPlaybackSource && audioPlayer.mediaStatus !== MediaPlayer.NoMedia && audioPlayer.error === MediaPlayer.NoError) {
            audioPlayer.position = target;
            return;
        }
        root.restorePlaybackPosition = target;
        root.audioPlaybackSource = root.originalAudioUrl();
    }
    function seekBy(milliseconds) {
        if (!root.voiceAudioReady || audioPlayer.duration <= 0) return;
        audioPlayer.position = Math.max(0, Math.min(audioPlayer.duration, audioPlayer.position + milliseconds));
        transcriptReader.follow();
    }
    function togglePlayback() {
        if (!root.voiceAudioReady) return;
        // A seek onto the very end leaves the player playing silence there; play again from the start.
        if (audioPlayer.duration > 0 && audioPlayer.position >= audioPlayer.duration - 250) {
            audioPlayer.position = 0; transcriptReader.follow(); root.playAudio(); return;
        }
        if (audioPlayer.playbackState === MediaPlayer.PlayingState) audioPlayer.pause();
        else { transcriptReader.follow(); root.playAudio(); }
    }
    function cancelCorrection() {
        root.clearCorrectionDraft();
        root.voiceDraftText = ''; root.correctionEdits = [];
        root.editingSegment = false;
    }
    function focusInEditable() {
        var edit = root.Window.activeFocusItem as TextEdit, input = root.Window.activeFocusItem as TextInput;
        return (!!edit && !edit.readOnly) || (!!input && !input.readOnly);
    }
    readonly property bool playbackKeysEnabled: visible && voiceAudioReady && !focusInEditable()
    Shortcut { sequence: 'Space'; enabled: root.playbackKeysEnabled; onActivated: root.togglePlayback() }
    Shortcut { sequence: 'Left'; enabled: root.playbackKeysEnabled; onActivated: root.seekBy(-5000) }
    Shortcut { sequence: 'Right'; enabled: root.playbackKeysEnabled; onActivated: root.seekBy(5000) }
    Shortcut { sequence: 'Shift+Left'; enabled: root.playbackKeysEnabled; onActivated: root.seekBy(-30000) }
    Shortcut { sequence: 'Shift+Right'; enabled: root.playbackKeysEnabled; onActivated: root.seekBy(30000) }
    Shortcut { sequence: '['; enabled: root.playbackKeysEnabled; onActivated: root.setPlaybackRate(TranscriptTiming.stepSpeed(root.playbackRate, -1, root.speedCeiling)) }
    Shortcut { sequence: ']'; enabled: root.playbackKeysEnabled; onActivated: root.setPlaybackRate(TranscriptTiming.stepSpeed(root.playbackRate, 1, root.speedCeiling)) }
    Shortcut { sequence: '0'; enabled: root.playbackKeysEnabled; onActivated: root.setPlaybackRate(1) }
    readonly property bool palettePlaybackReady: visible && voiceAudioReady
    onPalettePlaybackReadyChanged: Backend.voicePlaybackReady = palettePlaybackReady
    Component.onDestruction: Backend.voicePlaybackReady = false
    Connections {
        target: Backend
        function onVoicePlaybackRequestChanged() {
            if (!root.palettePlaybackReady) return;
            var action = Backend.voicePlaybackRequest.action;
            if (action === 'toggle') root.togglePlayback();
            else if (action === 'slower') root.setPlaybackRate(TranscriptTiming.stepSpeed(root.playbackRate, -1, root.speedCeiling));
            else if (action === 'faster') root.setPlaybackRate(TranscriptTiming.stepSpeed(root.playbackRate, 1, root.speedCeiling));
            else if (action === 'normal') root.setPlaybackRate(1);
        }
    }
    function selectPassage(start, end) {
        if (editingSegment) return; // a correction draft pins its exact target
        passageTarget = TranscriptDocument.target(transcriptDocument, start, end);
        selectedSegmentId = passageTarget ? passageTarget.id : '';
    }
    // Playback reads the original recording. Its loudness comes from the
    // engine's measured gain, applied to this player's own stream once it
    // plays: no copy of the recording, no control and no notice.
    property string audioPlaybackSource: ''
    property bool playbackGainApplied: false
    property bool playbackGainBusy: false
    readonly property string playbackStreams: Pipewire.nodes.values.filter(n => n.isStream).map(n => n.id).join(',')
    readonly property var playbackGainValue: selectedRecord ? selectedRecord.playback_gain : undefined
    onPlaybackStreamsChanged: playbackGainApplied = false
    onAudioPlaybackSourceChanged: playbackGainApplied = false
    onPlaybackGainValueChanged: playbackGainApplied = false
    Timer {
        // The first request goes when playback starts; it repeats only until the stream holds the gain.
        interval: 150; repeat: true; triggeredOnStart: true
        running: root.isVoice && audioPlayer.playbackState === MediaPlayer.PlayingState && !root.playbackGainApplied
        onTriggered: {
            if (root.playbackGainBusy) return;
            root.playbackGainBusy = true;
            var selection = [root.selectedId, root.audioPlaybackSource, root.playbackStreams];
            Backend.send('history-playback-gain', {id: root.selectedId, revision: root.selectedRecord.revision, player_pid: Quickshell.processId}, (value, ok) => {
                root.playbackGainBusy = false;
                if (ok && value && value.state === 'applied' && selection[0] === root.selectedId && selection[1] === root.audioPlaybackSource && selection[2] === root.playbackStreams) root.playbackGainApplied = true;
            }, true);
        }
    }
    property var captureSources: []
    property var captureCoverage: ({})
    property bool captureDetailsOpen: false
    property string captureNotice: ''
    readonly property int captureErrorCount: captureSources.filter(s => s.enabled && s.last_error).length

    readonly property bool hasSelection: !!selectedRecord && !!selectedRecord.id
    readonly property bool splitWide: width >= 980
    readonly property var segments: selectedRecord && selectedRecord.segments ? selectedRecord.segments : []
    readonly property var providerCards: [
        {key:'chatgpt', label:'ChatGPT export', detail:'conversations.json or explicit-role HTML'},
        {key:'codex', label:'Codex CLI', detail:'local JSONL sessions'},
        {key:'opencode', label:'OpenCode', detail:'read-only SQLite snapshot'},
        {key:'console', label:'Native console', detail:'local JSONL sessions'},
        {key:'voxtype', label:'Voxtype', detail:'completed recordings'}
    ]
    readonly property var selectedSegment: {
        if (!selectedSegmentId || !segments)
            return null;
        for (var i = 0; i < segments.length; i++)
            if (String(segments[i].id || '') === selectedSegmentId)
                return segments[i];
        return null;
    }

    Timer {
        id: searchTimer
        interval: 260
        repeat: false
        onTriggered: root.loadList(true)
    }

    Timer {
        // Fallback for a request that arrives while the view is settling; idle otherwise.
        id: externalSelectionTimer
        interval: 350
        repeat: true
        running: root.visible && root.hasExplicitNavigation()
        onTriggered: root.syncFromBackend()
    }

    Timer {
        interval: 30000
        repeat: true
        running: root.visible && Backend.available && !root.listBusy && !root.actionBusy && !root.editingSegment
        onTriggered: {
            root.loadCaptureSources();
            root.loadList(false);
        }
    }

    Timer {
        id: detailScrollRestoreTimer
        interval: 1
        repeat: false
        onTriggered: {
            detailScrollRetention.restore();
            transcriptScrollRetention.restore();
            transcriptReader.restore();
        }
    }

    function nextRequest() {
        return ++root.requestSerial;
    }

    function syncToBackend() {
        // Older shells do not expose the shared history selection yet. Keep
        // this optional so the view remains usable when embedded elsewhere.
        try {
            if (Backend.historySelectedId !== undefined)
                Backend.historySelectedId = root.selectedId;
            Backend.remember();
        } catch (e) {
            // An embedded host may provide no cross-navigation property.
        }
    }

    // Only an explicit source request moves the selection from outside. The
    // shared last-selected id is a stored mirror, never adopted by a view:
    // it may name another kind's record and would erase this view's own.
    function syncFromBackend() {
        if (!root.visible || Backend.page !== 'Strategy' || ['History','Chats','Voice notes'].indexOf(Backend.view) < 0) return;
        if (!hasExplicitNavigation()) return;
        var request = Backend.historyNavigation;
        consumedNavigationSerial = Number(request.serial);
        root.rememberKind();
        resolvingNavigationKind = true;
        root.selectRecord({id:String(request.id)}, true);
    }

    function loadList(resetOffset) {
        if (resetOffset)
            root.offset = 0;
        var token = root.nextRequest();
        root.listRequest = token;
        root.listBusy = true;
        root.errorText = '';
        Backend.send('history-list', {
            kind: root.kind,
            state: root.recordState,
            q: root.query,
            offset: root.offset,
            limit: root.limit
        }, (result, ok, error) => {
            if (!root || token !== root.listRequest)
                return;
            root.listBusy = false;
            if (!ok) {
                root.errorText = error || 'History could not be loaded.';
                return;
            }
            var value = result || {};
            root.items = value.items || [];
            root.total = Number(value.total || 0);
            root.offset = Number(value.offset === undefined ? root.offset : value.offset);
            root.limit = Number(value.limit || root.limit);
            if (root.offset >= root.total && root.offset > 0) { root.offset = Math.max(0, Math.floor((root.total-1)/root.limit)*root.limit); root.loadList(false); return; }
            root.retainSelection();
            root.queueTopics(root.items);
        });
    }

    // A chat's topic: its title when that is the person's own words, otherwise
    // the first line of its opening message, read once from the record and
    // kept for the session. Instruction blocks a harness injects are skipped.
    property var chatTopics: ({})
    property var topicQueue: []
    property bool topicBusy: false
    function instructionText(value) {
        return /^\s*(#\s*AGENTS\.md|<\/?[a-z_]*instructions>|<environment_context>|<permissions|<system|#\s*Codex Agent Instructions)/i.test(String(value || ''));
    }
    function topicLine(value) {
        var text = String(value || '').replace(/<([a-z_]+)[^>]*>[\s\S]*?<\/\1>/gi, ' ');
        var lines = text.split('\n');
        for (var i = 0; i < lines.length; ++i) {
            var line = lines[i].replace(/^[\s#>*-]+/, '').replace(/\s+/g, ' ').trim();
            if (!line || line.charAt(0) === '<') continue;
            var sentence = line.search(/[.!?](\s|$)/);
            if (sentence >= 24 && sentence < 140) line = line.slice(0, sentence + 1);
            return line.length > 160 ? line.slice(0, 159) + '…' : line;
        }
        return '';
    }
    function topicKey(record) { return String(record.id || '') + '@' + String(record.revision || ''); }
    function chatTopic(record) {
        if (!record || record.kind !== 'chat') return '';
        var title = String(record.title || '');
        if (title && !instructionText(title)) return topicLine(title);
        var known = chatTopics[topicKey(record)];
        return known !== undefined ? known : '';
    }
    function queueTopics(records) {
        topicQueue = (records || []).filter(r => r.kind === 'chat' && instructionText(r.title) && chatTopics[topicKey(r)] === undefined)
            .map(r => ({id: String(r.id), key: topicKey(r)}));
        pumpTopics();
    }
    function pumpTopics() {
        if (topicBusy || !topicQueue.length) return;
        var next = topicQueue[0];
        topicQueue = topicQueue.slice(1);
        topicBusy = true;
        Backend.send('history-detail', {id: next.id}, (detail, ok) => {
            root.topicBusy = false;
            if (ok && detail) {
                var segments = detail.segments || [], topic = '';
                var spoken = segments.filter(s => ['user', 'you', 'human'].indexOf(String(s.speaker || s.role || '').toLowerCase()) >= 0);
                [spoken, segments].forEach(list => {
                    for (var i = 0; i < list.length && !topic; ++i)
                        if (!root.instructionText(list[i].text)) topic = root.topicLine(list[i].text);
                });
                var topics = Object.assign({}, root.chatTopics);
                topics[next.key] = topic || 'No opening message recorded';
                root.chatTopics = topics;
            }
            root.pumpTopics();
        }, true);
    }
    function detailTitle(record) {
        if (record.kind === 'chat' && instructionText(record.title)) {
            var spoken = (record.segments || []).filter(s => !instructionText(s.text));
            return chatTopic(record) || (spoken.length ? topicLine(spoken[0].text) : '') || record.chat_kind_label || 'Conversation';
        }
        return record.title || 'Untitled record';
    }
    function providerLabel(provider) {
        var card = providerCards.find(c => c.key === provider);
        if (card) return card.label;
        var name = String(provider || 'Local source');
        return name.charAt(0).toUpperCase() + name.slice(1);
    }
    // Capture state words: what is switched on, and when a check last ran.
    function captureState(source) {
        return (source.enabled ? 'Enabled' : 'Disabled') + ' · '
            + (source.last_checked_at ? 'checked ' + formatDate(source.last_checked_at) : 'not checked');
    }
    function captureName(source) {
        var shared = captureSources.filter(s => s.provider === source.provider).length > 1;
        var folder = String(source.path || '').split('/').filter(part => part).pop() || '';
        return providerLabel(source.provider) + (shared && folder ? ' · ' + folder : '');
    }

    // A record hidden while its pane is open leaves the pane: the engine reads
    // it back empty with the mode on.
    function closeIfHidden() {
        if (!Backend.privateMode || !root.selectedId) return;
        var id = root.selectedId;
        Backend.send('history-detail', {id: id}, (r, ok) => {
            if (!root || root.selectedId !== id || !ok || (r && r.id)) return;
            // An open title edit on the hidden record is kept and shown again
            // when the record is next edited.
            if (labelDialog.visible && labelTarget && String(labelTarget.id) === String(id)) {
                var kept = Object.assign({}, labelDrafts);
                kept[id] = {title: labelTitle.text, purpose: labelPurpose.text};
                labelDrafts = kept; labelDialog.close();
            }
            stopAudio(); selectedId = ''; selectedRecord = null;
            selectedRevision = ''; revisions = []; detailFocused = false;
            detailRequest = nextRequest(); detailBusy = false;
            syncToBackend(); rememberKind();
        }, true);
    }

    function retainSelection() {
        // Paging or filtering the list must not invalidate an open citation.
        root.syncFromBackend();
    }

    property bool restoringCorrectionDraft: false
    property var correctionRevision: null
    function correctionDrafts() {
        var drafts = Object.assign({}, Backend.prefs.historyCorrectionDrafts || {});
        var legacy = Backend.prefs[root.correctionDraftKey] || {};
        if (legacy.record_id && legacy.segment_id && !drafts[legacy.segment_id])
            drafts[legacy.segment_id] = legacy;
        return drafts;
    }
    function persistCorrectionDraft() {
        if (root.restoringCorrectionDraft || !root.contextReady || !root.hasSelection || !root.selectedSegmentId || !root.editingSegment)
            return;
        var drafts = root.correctionDrafts();
        drafts[root.selectedSegmentId] = {
            record_id: root.selectedId, segment_id: root.selectedSegmentId,
            text: root.correctionText, reason: root.correctionReason,
            edits: root.correctionEdits, document_text: root.voiceDraftText,
            revision: root.correctionRevision
        };
        Backend.setting('historyCorrectionDrafts', drafts);
        var selected = Object.assign({}, Backend.prefs.historyCorrectionSegments || {});
        selected[root.selectedId] = root.selectedSegmentId;
        Backend.setting('historyCorrectionSegments', selected);
    }
    function clearCorrectionDraft(recordId, segmentId, submitted) {
        recordId = recordId || root.selectedId;
        segmentId = segmentId || root.selectedSegmentId;
        var drafts = root.correctionDrafts();
        var draft = drafts[segmentId];
        if (submitted && draft && (draft.record_id !== recordId || draft.text !== submitted.text || draft.document_text !== submitted.document_text || draft.reason !== submitted.reason || String(draft.revision) !== String(submitted.revision)))
            return false;
        delete drafts[segmentId];
        Backend.setting('historyCorrectionDrafts', drafts);
        var legacy = Backend.prefs[root.correctionDraftKey] || {};
        if (legacy.record_id === recordId && legacy.segment_id === segmentId)
            Backend.setting(root.correctionDraftKey, ({}));
        return true;
    }
    function restoreCorrectionDraft(segment) {
        if (!root.hasSelection) return false;
        var drafts = root.correctionDrafts();
        var segmentId = segment || (Backend.prefs.historyCorrectionSegments || {})[root.selectedId];
        if (!segmentId) {
            var legacy = Backend.prefs[root.correctionDraftKey] || {};
            if (legacy.record_id === root.selectedId) segmentId = legacy.segment_id;
        }
        var draft = drafts[segmentId];
        if (!draft || draft.record_id !== root.selectedId || !root.segments.some(s => String(s.id) === String(segmentId)))
            return false;
        root.restoringCorrectionDraft = true;
        root.selectedSegmentId = String(segmentId);
        root.correctionText = String(draft.text || '');
        root.correctionEdits = draft.edits || [];
        root.correctionReason = String(draft.reason || '');
        root.correctionRevision = draft.revision !== undefined ? draft.revision : root.selectedRecord.revision;
        root.editingSegment = true;
        if (root.isVoice)
            root.voiceDraftText = draft.document_text || TranscriptDocument.applyDraft(root.transcriptDocument, root.selectedSegmentId, root.correctionText);
        root.restoringCorrectionDraft = false;
        return true;
    }

    function selectRecord(summary, focusDetails) {
        if (!summary || !summary.id)
            return;
        root.persistCorrectionDraft();
        root.stopAudio();
        root.audioPlaybackSource = '';
        root.lastAudioError = '';
        root.detailFocused = focusDetails !== false;
        Backend.setting('historyDetailFocused', root.detailFocused);
        root.selectedId = String(summary.id);
        root.syncToBackend();
        root.selectedRecord = null;
        root.voiceDraftText = ''; root.correctionEdits = [];
        root.selectedRevision = '';
        root.revisions = [];
        root.selectedSegmentId = '';
        root.editingSegment = false;
        root.actionNotice = '';
        root.rememberKind();
        var token = root.nextRequest();
        root.detailRequest = token;
        root.detailBusy = true;
        root.errorText = '';
        Backend.send('history-detail', { id: summary.id }, (result, ok, error) => {
            if (!root || token !== root.detailRequest || root.selectedId !== String(summary.id))
                return;
            root.detailBusy = false;
            if (!ok) {
                resolvingNavigationKind = false;
                root.errorText = error || 'This history record could not be opened.';
                return;
            }
            var detail = result || summary;
            var actualKind = detail.kind || root.kind;
            if (actualKind !== root.kind) {
                // The record's own view opens on it with that view's search and page.
                var contexts = Object.assign({}, root.kindContexts);
                var prior = contexts[actualKind] || (Backend.prefs.recordViews || {})[actualKind] || {};
                contexts[actualKind] = {query:prior.query || '',offset:Number(prior.offset || 0),items:prior.items || [],
                    total:Number(prior.total || 0),selectedId:root.selectedId,selectedRecord:detail,
                    revisions:[],selectedRevision:String(detail.revision || ''),detailFocused:true};
                root.kindContexts = contexts;
                Qt.callLater(function() {
                    if (root.selectedId !== String(summary.id)) return;
                    root.recordKindResolved(actualKind);
                    if (root.kind === actualKind) { root.pendingResolvedKind = ''; return; }
                    // The person already left History, so the route cannot follow:
                    // this view keeps its own remembered selection, and the late
                    // record waits in its own view until History is shown again.
                    var own = root.kindContexts[root.kind] || {};
                    root.pendingResolvedKind = actualKind; root.resolvingNavigationKind = false;
                    root.selectedId = own.selectedId || ''; root.selectedRecord = own.selectedRecord || null;
                    root.revisions = own.revisions || []; root.selectedRevision = own.selectedRevision || '';
                    root.detailFocused = !!own.detailFocused;
                    root.syncToBackend();
                });
                return;
            }
            resolvingNavigationKind = false;
            root.selectedRecord = detail;
            root.rememberKind();
            root.selectedRevision = String(root.selectedRecord.revision || '');
            root.restoreCorrectionDraft();
            detailScrollRestoreTimer.restart();
            root.loadRevisions();
            Qt.callLater(function() { if (root.selectedId === String(summary.id)) root.recordKindResolved(actualKind); });
        });
    }

    function loadRevisions() {
        if (!root.hasSelection)
            return;
        var token = root.nextRequest();
        root.revisionRequest = token;
        var idAtRequest = root.selectedId;
        Backend.send('history-revisions', { id: root.selectedRecord.id }, (result, ok) => {
            if (!root || token !== root.revisionRequest || idAtRequest !== root.selectedId || !root.hasSelection)
                return;
            if (ok)
                root.revisions = (result || {}).items || [];
        });
    }

    function openRevision(value) {
        if (!value || !root.selectedRecord || String(value) === String(root.selectedRecord.revision || ''))
            return;
        var token = root.nextRequest();
        var idAtRequest = root.selectedId;
        root.detailRequest = token;
        root.detailBusy = true;
        root.errorText = '';
        Backend.send('history-detail', {
            id: idAtRequest,
            revision: value
        }, (result, ok, error) => {
            if (!root || token !== root.detailRequest || idAtRequest !== root.selectedId)
                return;
            root.detailBusy = false;
            if (!ok) {
                root.errorText = error || 'That revision could not be loaded.';
                return;
            }
            root.selectedRecord = result || root.selectedRecord;
            root.selectedRevision = String(root.selectedRecord.revision || value);
            root.selectedSegmentId = '';
            root.editingSegment = false;
            root.restoreCorrectionDraft();
            detailScrollRestoreTimer.restart();
        });
    }

    function startCorrection(segment) {
        if (!root.hasSelection || !segment) return;
        root.persistCorrectionDraft();
        root.editingSegment = false;
        if (root.restoreCorrectionDraft(String(segment.id))) return;
        root.selectedSegmentId = String(segment.id || '');
        root.correctionEdits = [];
        root.correctionText = String(segment.text || '');
        root.correctionReason = '';
        root.correctionRevision = root.selectedRecord.revision;
        root.editingSegment = true;
        root.actionNotice = '';
        root.persistCorrectionDraft();
    }

    function saveCorrection() {
        if (!root.hasSelection || !root.selectedSegment || (!root.correctionText.trim() && !root.correctionEdits.some(e => e.text.trim())))
            return;
        var token = root.nextRequest();
        var idAtRequest = root.selectedId;
        var segmentAtRequest = root.selectedSegmentId;
        var submitted = {text:root.correctionText, document_text:root.voiceDraftText, reason:root.correctionReason, revision:root.correctionRevision};
        root.actionRequest = token;
        root.actionBusy = true;
        root.errorText = '';
        Backend.send('history-correct', {
            id: root.selectedRecord.id,
            expected_revision: root.correctionRevision,
            segment_id: root.selectedSegment.id,
            edits: root.correctionEdits.length ? root.correctionEdits : undefined,
            text: root.correctionText,
            author: 'user',
            reason: root.correctionReason
        }, (result, ok, error) => {
            if (!root || token !== root.actionRequest)
                return;
            root.actionBusy = false;
            if (!ok) {
                root.errorText = error || 'The correction could not be saved.';
                return;
            }
            var cleared = root.clearCorrectionDraft(idAtRequest, segmentAtRequest, submitted);
            if (idAtRequest !== root.selectedId) {
                root.loadList(false);
                return;
            }
            root.selectedRecord = result || root.selectedRecord;
            root.selectedRevision = String(root.selectedRecord.revision || '');
            if (cleared && root.selectedSegmentId === segmentAtRequest) { root.editingSegment = false; root.voiceDraftText = ''; root.correctionEdits = []; }
            root.actionNotice = 'Transcript edit saved.';
            root.loadRevisions();
            root.loadList(false);
        });
    }

    function updateVoiceDraft(value) {
        root.voiceDraftText = value;
        var edit = root.voiceEdit;
        if (!edit) { root.clearCorrectionDraft(); root.editingSegment=false; root.correctionEdits=[]; return; }
        if (edit.invalid) return;
        root.selectedSegmentId = edit.id;
        root.correctionText = edit.text;
        root.correctionEdits = edit.edits || [];
        if (!root.editingSegment) root.correctionRevision = root.selectedRecord.revision;
        root.editingSegment = true;
        root.persistCorrectionDraft();
    }

    function importPathNow(sessionId) {
        var path = root.importPath.trim();
        if (!path) {
            root.errorText = 'Enter a file or directory path to import.';
            return;
        }
        var token = root.nextRequest();
        root.actionRequest = token;
        root.actionBusy = true;
        root.errorText = '';
        root.actionNotice = '';
        Backend.send('history-import', {
            path: path,
            provider: root.importProvider, session_id: sessionId || undefined
        }, (result, ok, error) => {
            if (!root || token !== root.actionRequest)
                return;
            root.actionBusy = false;
            if (!ok) {
                root.errorText = error || 'The history source could not be imported.';
                return;
            }
            var value = result || {};
            root.actionNotice = value.unchanged ? 'Already archived; no duplicate was created.' : 'Imported into History.';
            root.loadList(true);
            if (value.record && value.record.id)
                root.selectRecord(value.record);
        });
    }

    function loadCaptureSources() {
        Backend.send('history-capture-list', {}, (value, ok, error) => {
            if (!root) return;
            if (!ok) { root.errorText = error || 'Capture coverage could not be loaded.'; return; }
            root.captureSources = value.sources || [];
            root.captureCoverage = value.coverage || {};
        }, true);
    }

    function rememberCaptureSource() {
        var path = root.importPath.trim();
        if (!path) { root.errorText = 'Choose a local export or recording path first.'; return; }
        root.actionBusy = true;
        Backend.send('history-capture-add', {provider:root.importProvider, path:path}, (value, ok, error) => {
            root.actionBusy = false;
            if (!ok) { root.errorText = error || 'This local source could not be remembered.'; return; }
            root.errorText = '';
            root.actionNotice = 'Source remembered. Automatic checks run while Soliton is open.';
            root.loadCaptureSources();
        });
    }

    function syncCapture(id) {
        root.actionBusy = true;
        Backend.send('history-capture-sync', id ? {id:id} : {}, (value, ok, error) => {
            root.actionBusy = false;
            if (!ok) { root.errorText = error || 'The local source could not be checked.'; return; }
            root.errorText = '';
            var failures = (value.items || []).filter(item => item.ok === false);
            root.actionNotice = failures.length ? '' : 'Capture checked; unchanged source bytes were deduplicated.';
            root.captureNotice = failures.length ? failures.map(item => item.provider + ': ' + (item.error || ((item.errors || []).length + ' source items need attention'))).join('\n') : '';
            root.loadCaptureSources();
            root.loadList(true);
        });
    }

    function hasEnabledCapture() {
        for (var i = 0; i < root.captureSources.length; ++i)
            if (root.captureSources[i].enabled)
                return true;
        return false;
    }

    function setCaptureEnabled(source, enabled) {
        root.actionBusy = true;
        Backend.send(enabled ? 'history-capture-add' : 'history-capture-remove', enabled ? {provider: source.provider, path: source.path} : {id: source.id}, (value, ok, error) => {
            root.actionBusy = false;
            root.captureNotice = ok ? '' : (error || 'Capture setting could not be saved.');
            root.loadCaptureSources();
        }, true);
    }

    function discoverVoxtype() {
        var path = root.importPath.trim();
        if (!path) {
            root.errorText = 'Enter the Voxtype recording directory first.';
            return;
        }
        var token = root.nextRequest();
        root.actionRequest = token;
        root.actionBusy = true;
        root.errorText = '';
        root.actionNotice = '';
        Backend.send('history-discover', {
            path: path,
            provider: 'voxtype',
            limit: 200
        }, (result, ok, error) => {
            if (!root || token !== root.actionRequest)
                return;
            root.actionBusy = false;
            if (!ok) {
                root.errorText = error || 'Voxtype discovery failed.';
                return;
            }
            var value = result || {};
            var imported = Array.isArray(value.imported)?value.imported.length:Number(value.imported || 0);
            var unchanged = Array.isArray(value.unchanged)?value.unchanged.length:Number(value.unchanged || 0);
            var errors = Array.isArray(value.errors)?value.errors.length:Number(value.errors || 0);
            root.actionNotice = 'Voxtype: ' + imported + ' imported · ' + unchanged + ' unchanged' + (errors ? ' · ' + errors + ' errors' : '');
            root.loadList(true);
        });
    }

    function discoverSelectedProvider() {
        if (root.importProvider === 'voxtype') { root.discoverVoxtype(); return; }
        var path = root.importPath.trim();
        if (!path) { root.errorText = 'Choose a local source location first.'; return; }
        var token = root.nextRequest(); root.actionRequest = token; root.actionBusy = true; root.errorText = ''; root.actionNotice = '';
        Backend.send('history-discover', {path:path, provider:root.importProvider}, (result, ok, error) => {
            if (!root || token !== root.actionRequest) return;
            root.actionBusy = false;
            if (!ok) { root.errorText = error || 'Source discovery failed.'; return; }
            var value = result || {}, imported = Array.isArray(value.imported) ? value.imported.length : 0, unchanged = Array.isArray(value.unchanged) ? value.unchanged.length : 0, errors = Array.isArray(value.errors) ? value.errors.length : 0;
            root.actionNotice = root.importProvider + ': ' + imported + ' imported · ' + unchanged + ' unchanged' + (errors ? ' · ' + errors + ' errors' : '');
            root.loadList(true);
        });
    }

    property var openCodeSessions: []
    property int openCodeOffset: 0
    function browseOpenCode(offset) {
        root.actionBusy=true;root.errorText='';
        Backend.send('history-opencode-sessions',{path:root.importPath,offset:offset,limit:50},(r,ok,e)=>{
            root.actionBusy=false;
            if(!ok){root.errorText=e;return;}
            root.openCodeSessions=r.items || [];root.openCodeOffset=offset;openCodeBrowser.open();
        });
    }
    SDialog {
        id:openCodeBrowser;parent:Overlay.overlay;anchors.centerIn:parent;modal:true
        title:'Choose an OpenCode conversation';width:Math.min(760,parent.width-40);height:Math.min(680,parent.height-50)
        background:Rectangle {color:Theme.panel;border.color:Theme.line;radius: Theme.cornerRadius}
        contentItem:ListView {
            clip:true;model:root.openCodeSessions;spacing:6
            delegate:SButton {
                required property var modelData
                width:ListView.view.width;text:modelData.title || modelData.id
                onClicked:{openCodeBrowser.close();root.importPathNow(modelData.id);}
            }
            ScrollBar.vertical:SScrollBar {}
        }
        footer:RowLayout {
            SButton{text:'Close';onClicked:openCodeBrowser.close()}
            Item{Layout.fillWidth:true}
            SButton{text:'Previous';enabled:!root.actionBusy && root.openCodeOffset>0;onClicked:root.browseOpenCode(Math.max(0,root.openCodeOffset-50))}
            SButton{text:'Next';enabled:!root.actionBusy && root.openCodeSessions.length===50;onClicked:root.browseOpenCode(root.openCodeOffset+50)}
        }
    }
    property var labelTarget: null
    property string labelError: ''
    property var labelDrafts: ({})
    function openTitleEditor() {
        if (!selectedRecord) return;
        labelTarget = {id:selectedRecord.id, revision:selectedRecord.revision,
            title:selectedRecord.title || '', purpose:selectedRecord.purpose || ''};
        labelTitle.text = labelTarget.title; labelPurpose.text = labelTarget.purpose;
        var kept = labelDrafts[labelTarget.id];
        if (kept) {
            labelTitle.text = kept.title; labelPurpose.text = kept.purpose;
            var rest = Object.assign({}, labelDrafts); delete rest[labelTarget.id]; labelDrafts = rest;
        }
        labelError = ''; labelDialog.open();
    }
    function saveLabel() {
        if (!labelTarget || actionBusy || !labelTitle.text.trim()) return;
        var target = labelTarget, title = labelTitle.text, purpose = labelPurpose.text;
        actionBusy = true; labelError = '';
        Backend.send('history-label', {id:target.id,expected_revision:target.revision,title:title,purpose:purpose}, (r,ok,e) => {
            actionBusy = false;
            if (!ok) { labelError = e || 'The title could not be saved.'; return; }
            if (selectedId === String(target.id)) { selectedRecord = r; loadRevisions(); }
            loadList(false);
            if (labelTarget && String(labelTarget.id) === String(target.id)
                    && String(labelTarget.revision) === String(target.revision))
                labelDialog.close();
        }, true);
    }
    SDialog {
        id:labelDialog;parent:Overlay.overlay;anchors.centerIn:parent;modal:true
        title:'Edit title';width:Math.min(560,parent.width-40)
        dirty: !!root.labelTarget && (labelTitle.text !== root.labelTarget.title || labelPurpose.text !== root.labelTarget.purpose)
        onDirtyDismissalRequested: if (!root.actionBusy) labelCloseGuard.open()
        onClosed: { labelCloseGuard.close(); root.labelTarget = null; root.labelError = ''; }
        contentItem:ColumnLayout {
            LabelText{text:'Title';color:Theme.muted}
            Field{id:labelTitle;objectName:'history-title-field';Layout.fillWidth:true;Accessible.name:'History title'}
            LabelText{text:'Purpose';color:Theme.muted}
            Field{id:labelPurpose;objectName:'history-purpose-field';Layout.fillWidth:true;placeholderText:'What this conversation or note is for';Accessible.name:'History purpose'}
            LabelText{Layout.fillWidth:true;visible:!!root.labelError;text:root.labelError;color:Theme.dangerText;wrapMode:Text.Wrap}
        }
        footer:RowLayout {
            SButton{text:'Cancel';enabled:!root.actionBusy;onClicked:labelDialog.close()}
            Item{Layout.fillWidth:true}
            SButton{text:'Save';enabled:!root.actionBusy && !!labelTitle.text.trim();onClicked:root.saveLabel()}
        }
    }
    SDialog {
        id:labelCloseGuard;parent:Overlay.overlay;anchors.centerIn:parent;modal:true
        title:'Save changes?';width:Math.min(420,parent.width-32)
        contentItem:LabelText{text:'Keep the edited title before closing?';wrapMode:Text.Wrap}
        footer:RowLayout {
            SButton{text:'Cancel';onClicked:labelCloseGuard.close()}
            SButton{text:'Discard';onClicked:{labelCloseGuard.close();labelDialog.close();}}
            SButton{text:'Save';primary:true;onClicked:{labelCloseGuard.close();root.saveLabel();}}
        }
    }
    function formatDate(value) {
        if (value === undefined || value === null || value === '')
            return 'Unknown';
        var date;
        if (typeof value === 'number')
            date = new Date(value > 100000000000 ? value : value * 1000);
        else
            date = new Date(value);
        if (isNaN(date.getTime()))
            return String(value);
        return Qt.formatDateTime(date, 'MMM d, yyyy · h:mm ap');
    }

    function formatDuration(value) {
        if (value === undefined || value === null || value === '')
            return '';
        var total = Math.max(0, Math.round(Number(value) / 1000));
        var hours = Math.floor(total / 3600);
        var minutes = Math.floor((total % 3600) / 60);
        var seconds = total % 60;
        if (hours)
            return hours + ':' + (minutes < 10 ? '0' : '') + minutes + ':' + (seconds < 10 ? '0' : '') + seconds;
        return minutes + ':' + (seconds < 10 ? '0' : '') + seconds;
    }

    function audioReason(audio) {
        if (!audio)
            return 'No audio reference was archived.';
        return audio.reason || 'The original audio is not available.';
    }

    function selectedAudioPath() {
        return root.selectedRecord && root.selectedRecord.audio && root.selectedRecord.audio.path
            ? String(root.selectedRecord.audio.path) : '';
    }

    function stopAudio() {
        if (audioPlayer) audioPlayer.stop();
    }

    function originalAudioUrl() {
        var path = root.selectedAudioPath();
        return path ? 'file://' + path.split('/').map(encodeURIComponent).join('/') : '';
    }

    function playAudio() {
        var source = root.originalAudioUrl();
        if (!source) return;
        root.lastAudioError = '';
        if (root.audioPlaybackSource !== source) root.audioPlaybackSource = source;
        audioPlayer.play();
    }

    function sourceSummary(record) {
        if (!record || !record.source_refs || !record.source_refs.length)
            return 'No source reference was archived.';
        return record.source_refs.map(ref => {
            if (typeof ref === 'string')
                return ref;
            var provider = ref.provider ? ref.provider + ' · ' : '';
            var path = ref.path || ref.origin_id || 'Source reference';
            return provider + path + (ref.url ? '\n'+ref.url : '') + (ref.parent_id ? '\nRecorded parent: '+ref.parent_id : '');
        }).join('\n');
    }

    function relatedSummary(record) {
        if (!record || !record.related || !record.related.length)
            return '';
        return record.related.map(value => {
            if (typeof value === 'string')
                return value;
            return value.title || value.id || value.kind || 'Related record';
        }).join('\n');
    }

    function recordContext(record) {
        if (!record)
            return 'History';
        if (record.purpose && String(record.purpose).toLowerCase() !== 'unclassified')
            return String(record.purpose);
        if (record.kind === 'voice_note')
            return 'Voice note';
        if (record.kind === 'chat')
            return 'Conversation';
        if (record.kind === 'run')
            return 'Run';
        return 'Archived work';
    }

    // A chat's colour is the theme colour its kind's token names in the
    // mapping (grey is the theme's muted grey; purple is magenta in palettes
    // that name no purple).
    function tokenColor(token) {
        var name = String(token || '');
        if (!name || name === 'grey' || name === 'gray' || name === 'muted') return Theme.muted;
        if (name === 'green') return Theme.green;
        if (name === 'accent') return Theme.accent;
        return Theme.colors[name] || (name === 'purple' ? Theme.colors.magenta : '') || Theme.muted;
    }
    function agentColor(record) {
        if (!record) return Theme.muted;
        if (record.kind === 'voice_note') return Theme.accent;
        if (record.kind === 'run') return Theme.green;
        if (record.kind === 'chat') return tokenColor(record.chat_color_token);
        return Theme.muted;
    }

    function agentBadge(record) {
        // Purpose and lineage are separate facts. A title containing "plan"
        // or a recorded parent does not establish which tool ran the work.
        return root.recordContext(record);
    }

    Connections {
        target: Backend
        function onPrivacyApplied() { root.loadList(false); root.closeIfHidden(); }
    }
    Component.onCompleted: {
        root.contextReady = true;
        root.activateKind(root.requestedKind, true);
        root.loadCaptureSources();
    }
    onVisibleChanged: {
        if (!contextReady) return;
        if (!visible) { rememberKind(); if (editingSegment) persistCorrectionDraft(); audioPlayer.pause(); }
        else if (Backend.available && !editingSegment) {
            var waiting = pendingResolvedKind; pendingResolvedKind = '';
            // Followed after the navigation that showed this view has finished.
            if (waiting && !hasExplicitNavigation()) Qt.callLater(function() { if (root.visible && !root.hasExplicitNavigation()) root.recordKindResolved(waiting); });
            root.syncFromBackend(); root.loadList(false); root.loadCaptureSources();
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 12









        SplitView {
            id: split
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Horizontal
            handle: SSplitHandle { horizontal: true }

            Item {
                id: listPane
                objectName: 'history-list-pane'
                visible: root.splitWide || !root.detailFocused
                SplitView.fillWidth: !root.splitWide
                SplitView.minimumWidth: root.splitWide ? 330 : -1
                SplitView.preferredWidth: root.splitWide ? Math.min(400, Math.max(330, split.width * 0.32)) : -1
                SplitView.minimumHeight: -1
                SplitView.preferredHeight: -1

                ColumnLayout {
                    anchors.fill: parent
                    spacing: 8
        STabBar {
            objectName: 'history-collections'
            Layout.fillWidth: true
            Accessible.name: 'History collections'
            Repeater {
                model: root.collections
                delegate: STabButton {
                    required property var modelData
                    text: modelData.label
                    active: root.kind === modelData.kind
                    onClicked: root.openCollection(modelData.kind)
                }
            }
            Item { Layout.fillWidth: true }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            SSearchField {
                id: searchField
                objectName: 'history-search'
                Layout.fillWidth: true
                frameless: true
                scopeName: root.collectionLabel(root.kind).toLowerCase()
                text: root.query
                Accessible.name: 'Search ' + root.collectionLabel(root.kind).toLowerCase()
                onTextEdited: {
                    root.query = text;
                    searchTimer.restart();
                }
            }
            SButton { iconName: 'more'; accessibleLabel: 'Import and capture options'; quiet: true; onClicked: captureDialog.open() }
            Choice {
                objectName: 'history-record-visibility'
                // Narrow enough that the search keeps its width beside it.
                Layout.preferredWidth: 124
                model: ['Active', 'Archive', 'Trash']
                Accessible.name: 'Record visibility'
                onActivated: { root.recordState = ['active','archived','deleted'][currentIndex]; root.loadList(true); }
            }

        }
                    LabelText {
                        text: root.listBusy ? '' : root.total + (root.total === 1 ? ' record' : ' records')
                        color: Theme.muted
                        font.pixelSize: 12
                        font.letterSpacing: 0
                    }
                    ListView {
                        id: historyList
            SCollectionWheel { flickable: historyList }
                        objectName: 'history-list'
                        RetainedScroll { flickable: historyList; positionKey: 'history:list:' + root.kind }
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        spacing: 0
                        model: root.items
                        keyNavigationEnabled: true
                        activeFocusOnTab: true
                        Accessible.name: root.collectionLabel(root.kind)
                        delegate: CollectionRow {
                            id: summaryCard
                            ToolTip.visible: false
                            required property var modelData
                            width: historyList.width
                            readonly property bool chat: modelData.kind === 'chat'
                            heading: chat ? (root.chatTopic(modelData) || modelData.chat_kind_label || 'Conversation') : (modelData.title || 'Untitled record')
                            summary: chat ? (modelData.chat_kind_label || 'Conversation') : (modelData.preview || modelData.excerpt || modelData.summary || 'No preview was archived.')
                            // A voice note's line is its date and real length; other records keep their source and kind.
                            metadata: modelData.kind === 'voice_note'
                                ? [root.formatDate(modelData.captured_at || modelData.started_at), Number(modelData.duration_ms) > 0 ? root.formatDuration(modelData.duration_ms) : ''].filter(part => !!part).join(' · ')
                                : modelData.kind === 'run' ? root.formatDate(modelData.captured_at || modelData.started_at)
                                : (chat ? root.providerLabel(modelData.provider) : root.agentBadge(modelData)) + ' · ' + root.formatDate(modelData.captured_at || modelData.started_at)
                            marker: root.agentColor(modelData)
                            symbol: chat ? '' : modelData.kind === 'voice_note' ? 'play' : 'console'
                            // The chat's kind as a swatch in the row's icon place.
                            Rectangle {
                                visible: summaryCard.chat
                                x: 14; y: 20; width: 12; height: 12; radius: 6
                                color: root.agentColor(summaryCard.modelData)
                                Accessible.ignored: true
                            }
                            selected: root.selectedId === String(modelData.id || '')
                            onClicked: root.selectRecord(modelData)
                            onDoubleClicked: root.selectRecord(modelData)
                        }
                        ScrollBar.vertical: SScrollBar {}
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        visible: root.total > root.limit
                        Repeater {
                            model: Math.min(7, Math.ceil(root.total / root.limit))
                            delegate: SButton {
                                required property int index
                                readonly property int page: Math.max(1, Math.min(Math.ceil(root.total / root.limit) - 6,
                                    Math.floor(root.offset / root.limit) - 2)) + index
                                text: String(page)
                                primary: page === Math.floor(root.offset / root.limit) + 1
                                enabled: !root.listBusy
                                accessibleLabel: 'History page ' + page
                                onClicked: { root.offset = (page - 1) * root.limit; root.loadList(false); }
                            }
                        }
                        LabelText { Layout.fillWidth: true; text: 'Page ' + (Math.floor(root.offset / root.limit) + 1) + ' of ' + Math.ceil(root.total / root.limit); color: Theme.muted }
                    }
                }
            }

            Item {
                id: detailPane
                objectName: 'history-detail-pane'
                visible: root.splitWide || root.detailFocused
                SplitView.minimumWidth: root.splitWide ? 420 : -1
                SplitView.minimumHeight: -1
                SplitView.fillWidth: true
                SplitView.fillHeight: true

                SButton {
                    id: historyBack
                    anchors.top: parent.top
                    anchors.left: parent.left
                    visible: !root.splitWide
                    iconName: 'left'; text: root.collectionLabel(root.kind); quiet: true
                    onClicked: { root.detailFocused = false; Backend.setting('historyDetailFocused', false); }
                }
                ColumnLayout {
                    anchors.fill: parent
                    anchors.topMargin: historyBack.visible ? historyBack.height + 9 : 0
                    spacing: 9
                    visible: root.hasSelection

                    RowLayout {
                        Layout.fillWidth: true
                        LabelText {
                            Layout.fillWidth: true
                            text: root.selectedRecord ? root.detailTitle(root.selectedRecord) : ''
                            font.pixelSize: 26
                            font.weight: Font.DemiBold
                            font.letterSpacing: -0.5
                            elide: Text.ElideRight
                        }
                        SButton { id: recordActionsButton; objectName: 'history-record-actions'; text: ''; iconName: 'more'; accessibleLabel: 'Record actions'; onClicked: recordActions.open() }



                    }

                    RowLayout {
                        Layout.fillWidth: true
                        LabelText {
                            text: root.selectedRecord ? root.formatDate(root.selectedRecord.captured_at || root.selectedRecord.started_at) + (root.selectedRecord.purpose && String(root.selectedRecord.purpose).toLowerCase() !== 'unclassified' ? ' · ' + root.selectedRecord.purpose : '') : ''
                            color: Theme.muted
                            font.pixelSize: 12
                            Layout.fillWidth: true
                        }
                        SButton { text: 'Source'; iconName: 'source'; accessibleLabel: 'Source and record details'; quiet: true; onClicked: sourceDialog.open() }

                    }

                            Rectangle {
                                Layout.fillWidth: true
                                visible: root.isVoice || (!!root.selectedRecord && !!root.selectedRecord.audio && root.selectedRecord.audio.state === 'retained' && !!root.selectedRecord.audio.path)
                                color: Theme.panel
                                radius: Theme.cornerRadius
                                border.color: Theme.line
                                implicitHeight: 56
                                ColumnLayout {
                                    id: playerColumn
                                    anchors.fill: parent
                                    anchors.margins: 10
                                    spacing: 7

                                    RowLayout {
                                        Layout.fillWidth: true
                                        SButton {
                                            id: audioButton
                                            objectName: "history-audio-button"
                                            text: ''
                                            quiet: true
                                            focusPolicy: Qt.TabFocus
                                            accessibleLabel: audioPlayer.playbackState === MediaPlayer.PlayingState ? 'Pause' : 'Play'
                                            enabled: !!root.selectedAudioPath() && !!root.selectedRecord && !!root.selectedRecord.audio && root.selectedRecord.audio.state === 'retained'
                                            Layout.preferredWidth: Theme.iconTargetSize
                                            iconName: audioPlayer.playbackState === MediaPlayer.PlayingState ? 'pause' : 'play'
                                            onClicked: root.togglePlayback()
                                        }
                                        LabelText {
                                            visible: !root.selectedAudioPath()
                                            Layout.fillWidth: true; Layout.minimumWidth: 0
                                            text: 'Missing'; color: Theme.dangerText
                                            Accessible.description: root.audioReason(root.selectedRecord ? root.selectedRecord.audio : null)
                                        }
                                        SSlider {
                                            objectName: "history-audio-position"
                                            visible: !!root.selectedAudioPath()
                                            Layout.fillWidth: true
                                            from: 0
                                            to: Math.max(1, root.loadedDuration)
                                            value: root.loadedDuration > 0 ? audioPlayer.position : 0
                                            enabled: root.loadedDuration > 0
                                            onMoved: audioPlayer.position = value
                                            Accessible.name: 'Audio position'
                                        }
                                        LabelText { Layout.preferredWidth: 116; font.family: Theme.mono; text: root.formatDuration(root.loadedDuration > 0 ? audioPlayer.position : 0) + ' / ' + (root.loadedDuration > 0 ? root.formatDuration(root.loadedDuration) : root.selectedRecord && Number(root.selectedRecord.duration_ms) > 0 ? root.formatDuration(root.selectedRecord.duration_ms) : ''); color: Theme.muted; font.pixelSize: 12 }
                                        SButton {
                                            objectName: "history-audio-speed"
                                            focusPolicy: Qt.TabFocus
                                            visible: !!root.selectedAudioPath()
                                            text: TranscriptTiming.speedLabel(root.playbackRate)
                                            quiet: true
                                            accessibleLabel: 'Speed ' + text
                                            onClicked: root.setPlaybackRate(TranscriptTiming.nextSpeed(root.playbackRate, root.speedCeiling))
                                        }
                                    }
                                    MediaPlayer {
                                        id: audioPlayer
                                        audioOutput: AudioOutput { volume: 1.0 }
                                        playbackRate: root.playbackRate
                                        pitchCompensation: root.pitchKept
                                        // Do not probe or load retained WAVs while merely
                                        // browsing History. Play loads the original file.
                                        source: root.audioPlaybackSource
                                        onMediaStatusChanged: { if (mediaStatus === MediaPlayer.LoadedMedia && root.restorePlaybackPosition > 0) { position = root.restorePlaybackPosition; root.restorePlaybackPosition = 0; } }
                                        onErrorChanged: if (error !== MediaPlayer.NoError) root.lastAudioError = 'Failed'
                                        onPlaybackStateChanged: if (playbackState === MediaPlayer.PlayingState) root.hasPlayed = true
                                    }


                                }
                            }
                    TranscriptReader {
                        id: transcriptReader
                        objectName: 'voice-document-reader'
                        visible: root.isVoice && text.length > 0
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        text: root.voiceDraftText || root.transcriptDocument.text || (root.selectedRecord ? root.selectedRecord.original_text || '' : '')
                        document: root.transcriptDocument
                        placed: root.placedTimings
                        timingAvailable: !!root.wordTimings && !root.wordTimingFailed
                        audioAvailable: root.voiceAudioReady
                        marksShown: root.voiceAudioReady && root.hasPlayed && !!root.audioPlaybackSource && !root.correcting
                        playing: audioPlayer.playbackState === MediaPlayer.PlayingState
                        positionMs: audioPlayer.position
                        editable: root.transcriptDocument.spans.length > 0 && !root.actionBusy
                        onEdited: value => root.updateVoiceDraft(value)
                        onSeekRequested: milliseconds => root.seekTo(milliseconds)
                        title: 'Transcript: ' + (root.selectedRecord ? root.selectedRecord.title || '' : '')
                        positionKey: 'history:voice:' + root.selectedId
                        onPassageChanged: (start, end) => root.selectPassage(start, end)
                        actions: [
                            LabelText { text: 'Transcript'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 70 },
                            SButton { text: 'Cancel'; quiet: true
                                visible: !!root.voiceEdit
                                enabled: !root.actionBusy
                                onClicked: root.cancelCorrection() },
                            SButton { objectName: 'history-transcript-save'; text: 'Save'; primary: true; busy: root.actionBusy;
                                visible: !!root.voiceEdit && !root.voiceEdit.invalid
                                enabled: !root.actionBusy
                                onClicked: root.saveCorrection() }
                        ]
                    }
                    ScrollView {
                        id: detailScroll
                        objectName: "history-detail-scroll"
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        contentWidth: availableWidth
                        visible: root.hasSelection && !root.isVoice
                        RetainedScroll {
                            id: detailScrollRetention
                            flickable: detailScroll.contentItem
                            positionKey: 'history:detail:' + root.selectedId
                        }

                        ColumnLayout {
                            width: detailScroll.availableWidth
                            spacing: 12









                            Repeater {
                                model: root.runLines
                                delegate: RowLayout {
                                    id: runLine
                                    required property var modelData
                                    Layout.fillWidth: true
                                    spacing: 12
                                    LabelText { text: runLine.modelData.label; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 78; Layout.alignment: Qt.AlignTop }
                                    TextEdit { Layout.fillWidth: true; readOnly: true; selectByMouse: true; wrapMode: TextEdit.Wrap; textFormat: TextEdit.PlainText
                                        text: runLine.modelData.text; color: Theme.text; font.family: Theme.font; font.pixelSize: 14; Accessible.name: runLine.modelData.label }
                                }
                            }
                            RowLayout {
                                Layout.fillWidth: true
                                visible: !root.isRun
                                LabelText {
                                    text: 'Transcript'
                                    color: Theme.muted
                                    font.pixelSize: 12
                                    font.letterSpacing: 0
                                    Layout.fillWidth: true
                                }
                                LabelText {
                                    text: root.segments.length ? root.segments.length + ' segments' : 'No transcript segments archived.'
                                    color: Theme.muted
                                    font.pixelSize: 12
                                }
                            }
                            ListView {
                                id: transcriptList
                                visible: !root.isRun
                                RetainedScroll {
                                    id: transcriptScrollRetention
                                    flickable: transcriptList
                                    positionKey: 'history:transcript:' + root.selectedId
                                }
                                Layout.fillWidth: true
                                Layout.preferredHeight: Math.max(150, Math.min(560, root.segments.length * 86))
                                clip: true
                                spacing: 7
                                model: root.isVoice ? [] : root.segments
                                reuseItems: true
                                ScrollBar.vertical: SScrollBar {}
                                delegate: Rectangle {
                                    id: segmentCard
                                    required property var modelData
                                    width: transcriptList.width - 12
                                    height: segmentColumn.implicitHeight + 20
                                    radius: Theme.cornerRadius
                                    color: root.selectedSegmentId === String(modelData.id || '') ? Theme.selection : Theme.panel
                                    border.color: root.selectedSegmentId === String(modelData.id || '') ? Theme.accent : Theme.line
                                    MouseArea {
                                        anchors.fill: parent
                                        onClicked: {
                                            root.selectedSegmentId = String(segmentCard.modelData.id || '');
                                            root.editingSegment = false;
                                        }
                                    }
                                    ColumnLayout {
                                        id: segmentColumn
                                        anchors.fill: parent
                                        anchors.margins: 10
                                        spacing: 5
                                        RowLayout {
                                            Layout.fillWidth: true
                                            LabelText {
                                                text: root.formatDuration(segmentCard.modelData.start_ms || 0)
                                                color: Theme.accent
                                                font.family: Theme.mono
                                                font.pixelSize: 12
                                            }
                                            LabelText {
                                                visible: segmentCard.modelData.end_ms !== undefined
                                                text: '→ ' + root.formatDuration(segmentCard.modelData.end_ms)
                                                color: Theme.muted
                                                font.family: Theme.mono
                                                font.pixelSize: 12
                                            }
                                            LabelText {
                                                text: segmentCard.modelData.speaker ? '· ' + segmentCard.modelData.speaker : ''
                                                color: Theme.muted
                                                font.pixelSize: 12
                                                Layout.fillWidth: true
                                            }
                                            SButton {
                                                text: 'Edit'
                                                quiet: true
                                                onClicked: root.startCorrection(segmentCard.modelData)
                                            }
                                        }
                                        Text {
                                            Layout.fillWidth: true
                                            text: segmentCard.modelData.text || ''
                                            textFormat: Text.PlainText
                                            color: Theme.text
                                            font.pixelSize: 13
                                            wrapMode: Text.Wrap
                                            Accessible.name: 'Transcript segment ' + (segmentCard.modelData.index !== undefined ? segmentCard.modelData.index + 1 : '')
                                        }
                                    }
                                }
                            }









                        }
                    }
                }

                ColumnLayout {
                    anchors.centerIn: parent
                    visible: !root.hasSelection
                    spacing: 6
                    LabelText {
                        text: !root.detailBusy && root.errorText ? 'Failed' : ''
                        color: Theme.dangerText
                        font.pixelSize: 16
                        Layout.alignment: Qt.AlignHCenter
                    }


                }
            }
        }
    }

    NoticeOverlay {
        anchors.right: parent.right; anchors.bottom: parent.bottom; anchors.margins: 12
        text: root.errorText || root.lastAudioError || root.actionNotice
        error: !!root.errorText || !!root.lastAudioError
        onDismissed: { root.errorText = ''; root.lastAudioError = ''; root.actionNotice = ''; }
    }
    SDialog {
        id: sourceDialog
        parent: Overlay.overlay; anchors.centerIn: parent; modal: true
        title: 'Source & record details'
        width: Math.min(700, parent.width - 32); height: Math.min(720, parent.height - 32)
        background: Rectangle { color: Theme.base; border.color: Theme.edge }
        contentItem: ScrollView {
            id: sourceScroll; clip: true; contentWidth: availableWidth
            ColumnLayout {
                width: sourceScroll.availableWidth; spacing: 16
                        Choice {
                            visible: root.revisions.length > 1
                            enabled: root.revisions.length > 0
                            Layout.preferredWidth: 150
                            model: root.revisions.map(r => 'Revision ' + (r.revision || r.id || '?'))
                            currentIndex: {
                                var index = -1;
                                for (var i = 0; i < root.revisions.length; i++)
                                    if (String(root.revisions[i].revision || root.revisions[i].id || '') === String(root.selectedRevision))
                                        index = i;
                                return index >= 0 ? index : 0;
                            }
                            Accessible.name: 'Transcript revision'
                            onActivated: {
                                var revision = root.revisions[currentIndex];
                                root.openRevision(revision.revision || revision.id);
                            }
                        }
                            ColumnLayout {
                                Layout.fillWidth:true
                                visible:!!root.selectedRecord && !!root.selectedRecord.relationships && (!!root.selectedRecord.relationships.parent_origin_id || (root.selectedRecord.relationships.children || []).length>0)
                                LabelText { Layout.fillWidth:true;wrapMode:Text.Wrap;color:Theme.muted;text:root.selectedRecord && root.selectedRecord.agent ? ((root.selectedRecord.agent.nickname || '')+' '+(root.selectedRecord.agent.path || '')) : '' }
                                SButton { visible:!!root.selectedRecord && !!root.selectedRecord.relationships && !!root.selectedRecord.relationships.parent; text:'Open parent conversation';onClicked:Backend.showHistory(root.selectedRecord.relationships.parent.id) }
                                LabelText { Layout.fillWidth:true;wrapMode:Text.Wrap;color:Theme.muted;visible:!!root.selectedRecord && !!root.selectedRecord.relationships && !!root.selectedRecord.relationships.parent_origin_id && !root.selectedRecord.relationships.parent;text:visible ? 'Recorded parent '+root.selectedRecord.relationships.parent_origin_id+' · not yet archived' : '' }
                                Repeater { model:root.selectedRecord && root.selectedRecord.relationships ? root.selectedRecord.relationships.children || [] : [];delegate:SButton {required property var modelData;text:'↳ '+modelData.title;onClicked:Backend.showHistory(modelData.id)} }
                            }
                            LabelText {
                                visible: !!root.selectedRecord && !!root.selectedRecord.related && root.selectedRecord.related.length > 0
                                text: 'RELATED WORK\n' + root.relatedSummary(root.selectedRecord)
                                color: Theme.muted
                                font.pixelSize: 12
                                wrapMode: Text.Wrap
                                Layout.fillWidth: true
                            }
                            Repeater {
                                model:root.selectedRecord?(root.selectedRecord.related || []).filter(r=>r.kind==='proposal'):[]
                                SButton {
                                    required property var modelData
                                    Layout.fillWidth:true;text:(modelData.stale?'Source changed · ':'')+(modelData.title || '')+' · '+(modelData.status || '')
                                    onClicked:Backend.reviewProposalRequested(modelData.id)
                                }
                            }
                Rectangle {
                    visible: !!root.selectedSegment
                    Layout.fillWidth: true; implicitHeight: passageColumn.implicitHeight + 24
                    color: Theme.panel; border.color: Theme.line
                    ColumnLayout {
                        id: passageColumn; anchors.fill: parent; anchors.margins: 12; spacing: 8
                        SectionLabel { text: 'Selected passage' }
                        LabelText { Layout.fillWidth: true; wrapMode: Text.Wrap; text: root.selectedSegment ? String(root.selectedSegment.id) + ' · ' + root.formatDuration(root.selectedSegment.start_ms || 0) + '–' + root.formatDuration(root.selectedSegment.end_ms) : ''; color: Theme.muted; font.pixelSize: 12 }
                        TextEdit { Layout.fillWidth: true; readOnly: true; selectByMouse: true; wrapMode: TextEdit.Wrap; textFormat: TextEdit.PlainText; text: root.selectedSegment ? root.selectedSegment.text || '' : ''; color: Theme.text; font.family: Theme.font; font.pixelSize: 16; Accessible.name: 'Original source passage' }
                        SButton { text: 'Edit this passage'; visible: !root.isVoice; enabled: !!root.selectedSegment && !root.actionBusy; onClicked: { sourceDialog.close(); root.startCorrection(root.selectedSegment); } }
                    }
                }
                            Rectangle {
                                Layout.fillWidth: true
                                color: Theme.panel
                                radius: Theme.cornerRadius
                                border.color: Theme.line
                                implicitHeight: overviewColumn.implicitHeight + 22
                                ColumnLayout {
                                    id: overviewColumn
                                    anchors.fill: parent
                                    anchors.margins: 11
                                    spacing: 6
                                    LabelText {
                                        text: 'OVERVIEW'
                                        color: Theme.muted
                                        font.pixelSize: 12
                                        font.letterSpacing: 0
                                    }
                                    RowLayout {
                                        Layout.fillWidth: true
                                        LabelText { text: 'Captured'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 78 }
                                        LabelText { text: root.formatDate(root.selectedRecord ? (root.selectedRecord.captured_at || root.selectedRecord.started_at) : ''); font.pixelSize: 12; Layout.fillWidth: true; wrapMode: Text.Wrap }
                                    }
                                    RowLayout {
                                        Layout.fillWidth: true
                                        LabelText { text: 'Imported'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 78 }
                                        LabelText { text: root.formatDate(root.selectedRecord ? root.selectedRecord.imported_at : ''); font.pixelSize: 12; Layout.fillWidth: true; wrapMode: Text.Wrap }
                                    }
                                    RowLayout {
                                        Layout.fillWidth: true
                                        LabelText { text: 'Duration'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 78 }
                                        LabelText { text: root.formatDuration(root.selectedRecord ? root.selectedRecord.duration_ms : undefined); font.pixelSize: 12; Layout.fillWidth: true }
                                    }
                                    RowLayout {
                                        Layout.fillWidth: true
                                        LabelText { text: 'Coverage'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 78 }
                                        LabelText {
                                            text: root.selectedRecord && root.selectedRecord.coverage ? (root.selectedRecord.coverage.state || 'Unknown') + (root.selectedRecord.coverage.reason ? ' · ' + root.selectedRecord.coverage.reason : '') : 'Unknown'
                                            color: root.selectedRecord && root.selectedRecord.coverage && root.selectedRecord.coverage.state === 'complete' ? Theme.green : Theme.muted
                                            font.pixelSize: 12
                                            wrapMode: Text.Wrap
                                            Layout.fillWidth: true
                                        }
                                    }
                                    RowLayout {
                                        Layout.fillWidth: true
                                        visible: !!root.selectedRecord && !!root.selectedRecord.audio
                                        LabelText { text: 'Audio'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 78 }
                                        LabelText {
                                            text: root.selectedRecord && root.selectedRecord.audio && root.selectedRecord.audio.state === 'retained' ? 'Retained' : root.audioReason(root.selectedRecord ? root.selectedRecord.audio : null)
                                            color: root.selectedRecord && root.selectedRecord.audio && root.selectedRecord.audio.state === 'retained' ? Theme.green : Theme.muted
                                            font.pixelSize: 12
                                            wrapMode: Text.Wrap
                                            Layout.fillWidth: true
                                        }
                                    }
                                }
                            }
                            LabelText {
                                visible: !!root.selectedRecord && !!root.selectedRecord.original_text
                                text: 'ORIGINAL TEXT'
                                color: Theme.muted
                                font.pixelSize: 12
                                font.letterSpacing: 0
                            }
                            Text {
                                visible: !!root.selectedRecord && !!root.selectedRecord.original_text
                                Layout.fillWidth: true
                                text: root.selectedRecord ? (root.selectedRecord.original_text || '') : ''
                                textFormat: Text.PlainText
                                color: Theme.text
                                font.family: Theme.mono
                                font.pixelSize: 12
                                wrapMode: Text.Wrap
                                Accessible.name: 'Original imported text'
                            }
                            Rectangle {
                                Layout.fillWidth: true
                                color: Theme.panel
                                radius: Theme.cornerRadius
                                border.color: Theme.line
                                implicitHeight: sourceColumn.implicitHeight + 22
                                ColumnLayout {
                                    id: sourceColumn
                                    anchors.fill: parent
                                    anchors.margins: 11
                                    spacing: 6
                                    LabelText { text: 'SOURCE'; color: Theme.muted; font.pixelSize: 12; font.letterSpacing: 0 }
                                    Text {
                                        Layout.fillWidth: true
                                        text: root.sourceSummary(root.selectedRecord)
                                        textFormat: Text.PlainText
                                        color: Theme.text
                                        font.family: Theme.mono
                                        font.pixelSize: 12
                                        wrapMode: Text.Wrap
                                        Accessible.name: 'Archived source references'
                                    }
                                }
                            }
                            Rectangle {
                                Layout.fillWidth: true
                                color: Theme.panel
                                radius: Theme.cornerRadius
                                border.color: Theme.line
                                implicitHeight: activityColumn.implicitHeight + 22
                                ColumnLayout {
                                    id: activityColumn
                                    anchors.fill: parent
                                    anchors.margins: 11
                                    spacing: 6
                                    LabelText { text: 'ACTIVITY'; color: Theme.muted; font.pixelSize: 12; font.letterSpacing: 0 }
                                    LabelText {
                                        Layout.fillWidth: true
                                        text: root.selectedRecord ? ('Revision ' + (root.selectedRecord.revision || 'unknown') + ' · imported ' + root.formatDate(root.selectedRecord.imported_at)) : ''
                                        color: Theme.text
                                        font.pixelSize: 12
                                        wrapMode: Text.Wrap
                                    }
                                    LabelText {
                                        visible: !!root.selectedRecord && !!root.selectedRecord.correction
                                        Layout.fillWidth: true
                                        text: root.selectedRecord && root.selectedRecord.correction ? ('Edited by ' + (root.selectedRecord.correction.author || 'user') + ' · ' + root.formatDate(root.selectedRecord.correction.created_at) + (root.selectedRecord.correction.reason ? '\n' + root.selectedRecord.correction.reason : '')) : ''
                                        color: Theme.muted
                                        font.pixelSize: 12
                                        wrapMode: Text.Wrap
                                    }
                                }
                            }
            }
        }
    }
    SDialog {
        id: correctionDialog
        parent: Overlay.overlay; anchors.centerIn: parent; modal: true
        title: 'Edit passage'
        width: Math.min(700, parent.width - 32); height: Math.min(560, parent.height - 32)
        background: Rectangle { color: Theme.base; border.color: Theme.edge }
        onClosed: { if (root.editingSegment) root.persistCorrectionDraft(); root.editingSegment = false; }
        contentItem: ScrollView {
            id: correctionDialogScroll; clip: true; contentWidth: availableWidth
            ColumnLayout { width: correctionDialogScroll.availableWidth
                            Rectangle {
                                Layout.fillWidth: true
                                visible: true
                                color: Theme.selection
                                radius: Theme.cornerRadius
                                border.color: Theme.accent
                                implicitHeight: correctionColumn.implicitHeight + 22
                                ColumnLayout {
                                    id: correctionColumn
                                    anchors.fill: parent
                                    anchors.margins: 11
                                    spacing: 7
                                    LabelText {
                                        text: 'CORRECT TRANSCRIPT · APPENDS A REVISION'
                                        color: Theme.accent
                                        font.pixelSize: 12
                                        font.letterSpacing: 1.0
                                    }
                                    ScrollView {
                                        id: correctionScroll
                                        Layout.fillWidth: true
                                        Layout.preferredHeight: 220
                                        clip: true
                                        contentWidth: availableWidth
                                        STextArea {
                                        id: correctionField
                                        font.family: Theme.font
                                        font.pixelSize: Theme.readerSize
                                        width: correctionScroll.availableWidth
                                        implicitWidth: 0
                                        text: root.correctionText
                                        color: Theme.text
                                        placeholderText: 'Corrected transcript text'
                                        placeholderTextColor: Theme.muted
                                        wrapMode: TextArea.Wrap
                                        selectByMouse: true
                                        background: Rectangle { color: Theme.base; radius: Theme.cornerRadius; border.color: correctionField.activeFocus ? Theme.accent : Theme.line }
                                        Accessible.name: 'Corrected transcript text'
                                        onTextChanged: {
                                            root.correctionText = text;
                                            root.persistCorrectionDraft();
                                        }
                                    }
                                    }
                                    Field {
                                        Layout.fillWidth: true
                                        placeholderText: 'Reason (optional)'
                                        text: root.correctionReason
                                        Accessible.name: 'Correction reason'
                                        onTextEdited: {
                                            root.correctionReason = text;
                                            root.persistCorrectionDraft();
                                        }
                                    }
                                    RowLayout {
                                        Layout.fillWidth: true
                                        Item { Layout.fillWidth: true }
                                        SButton {
                                            text: 'Cancel'
                                            quiet: true
                                            onClicked: {
                                                root.editingSegment = false;
                                                root.clearCorrectionDraft();
                                            }
                                        }
                                        SButton {
                busy: root.actionBusy

                                            text: 'Save edit'
                                            primary: true
                                            enabled: !root.actionBusy && root.correctionText.trim().length > 0
                                            onClicked: root.saveCorrection()
                                        }
                                    }
                                }
                            }
            }
        }
    }

    SPopup {
        // Outside click and Escape close it; every item is a record operation.
        id: recordActions; parent: Overlay.overlay; trigger: recordActionsButton; alignRight: true; padding: 8
        width: 238
        contentItem: ColumnLayout {
            spacing: 0
            MenuAction {
                text: 'Edit title'; enabled: root.hasSelection
                onClicked: { recordActions.close(); root.openTitleEditor(); }
            }
            MenuAction {
                text: root.selectedRecord && root.selectedRecord.record_state && root.selectedRecord.record_state !== 'active' ? 'Restore' : 'Archive'
                enabled: root.hasSelection && !root.actionBusy && !root.editingSegment
                onClicked: { recordActions.close(); root.changeRecordState(root.selectedRecord.record_state && root.selectedRecord.record_state !== 'active' ? 'active' : 'archived', false); }
            }
            MenuAction {
                text: 'Delete'
                enabled: root.hasSelection && root.selectedRecord.record_state !== 'deleted' && !root.actionBusy && !root.editingSegment
                onClicked: { root.lifecycleTarget = {id:root.selectedRecord.id, revision:root.selectedRecord.revision, title:root.selectedRecord.title}; recordActions.close(); deleteRecordDialog.open(); }
            }
            MenuAction {
                text: 'Propose work'
                enabled: root.hasSelection && !!root.selectedSegment && !root.actionBusy
                onClicked: { recordActions.close(); root.proposalRequested(root.selectedRecord, root.selectedSegment); }
            }
            MenuAction {
                readonly property bool marked: root.hasSelection && Backend.isMarkedPrivate(root.selectedRecord.id)
                text: marked ? 'Unmark private' : 'Mark private'
                visible: root.hasSelection && !!root.selectedRecord.id
                onClicked: { var id = root.selectedRecord.id, on = !marked; recordActions.close(); Backend.setMarkedPrivate(id, on); }
            }
        }
    }

    SDialog {
        id: deleteRecordDialog; parent: Overlay.overlay; anchors.centerIn: parent; modal: true
        title: 'Move record to Trash?'
        width: Math.min(480, parent.width - 32)
        standardButtons: Dialog.Cancel | Dialog.Ok
        onAccepted: root.changeRecordState('deleted', true)
        contentItem: LabelText {
            text: (root.lifecycleTarget ? root.lifecycleTarget.title : '') + '\n\nThis hides this record. Restore it from Trash. Original files, transcript revisions and corrections are retained.'
            wrapMode: Text.Wrap
        }
    }
    SDialog {
        id: captureDialog
        parent: Overlay.overlay; anchors.centerIn: parent; modal: true
        title: 'Import & capture'
        width: Math.min(660, parent.width-32); height: Math.min(690, parent.height-32)
        standardButtons: Dialog.Close
        contentItem: ScrollView {
            id: captureScroll; clip: true; contentWidth: availableWidth
            ColumnLayout {
                width: captureScroll.availableWidth; spacing: 14
                LabelText { Layout.fillWidth:true; text:'Choose an existing local source. Importing does not move or delete its files.'; color:Theme.muted; wrapMode:Text.Wrap }
                Choice { Layout.fillWidth:true; model:root.providerCards; textRole:'label'; currentIndex:Math.max(0,model.findIndex(p=>p.key===root.importProvider)); onActivated: index => root.importProvider = root.providerCards[index].key; Accessible.name:'Source provider' }
                Field { Layout.fillWidth:true; text:root.importPath; placeholderText:'Local source path'; onTextEdited:root.importPath=text; Accessible.name:'Local source path' }
                RowLayout {
                    Layout.fillWidth:true
                    SButton {text:'Import';primary:true;enabled:!root.actionBusy&&!!root.importPath.trim();onClicked:root.importPathNow('')}
                    SButton {text:'Discover';enabled:!root.actionBusy&&!!root.importPath.trim();onClicked:root.discoverSelectedProvider()}
                    SButton {text:'Conversations';visible:root.importProvider==='opencode';enabled:!root.actionBusy;onClicked:root.browseOpenCode(0)}
                }
                SDisclosure {
                    Layout.fillWidth:true; title:'Automatic capture'; summary:root.captureSources.length+' sources'
                    MenuAction {text:'Remember this source';iconName:'plus';enabled:!root.actionBusy&&!!root.importPath.trim();onClicked:root.rememberCaptureSource()}
                    MenuAction {text:'Check enabled sources';enabled:!root.actionBusy&&root.hasEnabledCapture();onClicked:root.syncCapture('')}
                    Repeater {
                        model:root.captureSources
                        ColumnLayout {
                            id:capture;required property var modelData;Layout.fillWidth:true
                            SCheckBox {Layout.fillWidth:true;text:root.captureName(capture.modelData);checked:!!capture.modelData.enabled;enabled:!root.actionBusy;onToggled:root.setCaptureEnabled(capture.modelData,checked)}
                            LabelText {Layout.fillWidth:true;Layout.leftMargin:30;text:root.captureState(capture.modelData);color:Theme.muted;font.pixelSize:12}
                            LabelText {Layout.fillWidth:true;visible:!!capture.modelData.last_error;text:capture.modelData.last_error||'';color:Theme.dangerText;wrapMode:Text.Wrap}
                        }
                    }
                }
                LabelText {Layout.fillWidth:true;visible:!!root.errorText||!!root.captureNotice;text:root.errorText||root.captureNotice;color:Theme.dangerText;wrapMode:Text.Wrap}
                LabelText {Layout.fillWidth:true;visible:!!root.actionNotice;text:root.actionNotice;color:Theme.muted;wrapMode:Text.Wrap}
            }
        }
    }

    // Automation only: what the page shows, read from its own items.
    IpcHandler {
        enabled: Quickshell.env('SOLITON_AUTOMATION') === '1'
        target: 'soliton-history'
        function tree(): string {
            var texts = [];
            function walk(item) {
                if (!item || !item.visible) return;
                if (typeof item.text === 'string' && item.text.length) texts.push(item.text);
                for (var i = 0; i < item.children.length; ++i) walk(item.children[i]);
            }
            walk(root);
            [captureDialog, recordActions, sourceDialog].forEach(popup => { if (popup.visible) { walk(popup.contentItem); if (popup.footer) walk(popup.footer); } });
            var collections = [];
            var bar = root.findItem(root, 'history-collections');
            function tabs(item) {
                if (!item) return;
                if (item.active !== undefined && typeof item.text === 'string' && item.visible) collections.push(item.text);
                for (var i = 0; i < item.children.length; ++i) tabs(item.children[i]);
            }
            tabs(bar);
            var editor = transcriptReader.editor;
            return JSON.stringify({kind: root.kind, selected: root.selectedId, collections: collections, texts: texts,
                paragraphs: transcriptReader.visible ? editor.text.split('\n\n').filter(p => p.trim()).length : 0,
                editorReadOnly: editor.readOnly, editorFocused: editor.activeFocus, cursor: editor.cursorPosition,
                editButtons: texts.filter(t => t === 'Edit').length, saveShown: texts.indexOf('Save') >= 0,
                rows: root.items.slice(0, 8).map(r => ({kind: r.chat_kind || r.kind, token: r.chat_color_token || '',
                    colour: String(root.agentColor(r)), topic: root.chatTopic(r)})),
                gainApplied: root.playbackGainApplied, position: audioPlayer.position, state: audioPlayer.playbackState,
                source: root.audioPlaybackSource, captureSources: root.captureSources.map(c => root.captureName(c) + ' · ' + root.captureState(c))});
        }
        function editorPoint(offset: int): string {
            var editor = transcriptReader.editor, rect = editor.positionToRectangle(offset);
            var point = editor.mapToItem(null, rect.x + 1, rect.y + rect.height / 2);
            return JSON.stringify({x: Math.round(point.x), y: Math.round(point.y)});
        }
        function openCapture(): void { captureDialog.open(); }
    }
    function findItem(item, name) {
        if (!item) return null;
        if (item.objectName === name) return item;
        for (var i = 0; i < item.children.length; ++i) { var found = findItem(item.children[i], name); if (found) return found; }
        return null;
    }
}
