import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Quickshell

// Local Learning Center adapters. The Rust store owns parsing and a single
// bounded worker; this view only submits explicit sources and polls receipts.
Item {
    id: root

    property var sources: []
    property var jobs: []
    property string kind: 'youtube-channel'
    readonly property bool schedulesEnabled: ['1', 'true', 'yes'].indexOf(Quickshell.env('SOLITON_COLLECTION_AUTOSTART')) >= 0
    property bool autoBriefing:false
    property string goalId:''
    property string location: ''
    property int cadenceHours: 24
    property string error: ''
    property bool busy: false
    property string pageCursor: ''
    property string nextCursor: ''
    property var cursorStack: []
    property int pageNumber: 0
    property string focusedJobId: ''
    property var focusedJob: null
    property bool focusedLoading: false

    function refresh(resetPage) {
        if (!Backend.available || !Backend.restored)
            return ;
        if (resetPage === true) {
            root.pageCursor = '';
            root.nextCursor = '';
            root.cursorStack = [];
            root.pageNumber = 0;
        }
        var args = {'limit': 50};
        if (root.pageCursor)
            args.cursor = root.pageCursor;
        Backend.send('collection-status', args, (result, ok, message) => {
            if (!root) return;
            if (!ok) {
                root.error = message || 'Collection status unavailable';
                return ;
            }
            root.error = '';
            root.sources = result.sources || [];
            root.jobs = result.jobs || [];
            root.nextCursor = String(result.next_cursor || '');
            root.lookupFocusedJob();
        }, true);
    }

    function nextPage() {
        if (!root.nextCursor) return;
        root.cursorStack = root.cursorStack.concat([root.pageCursor]);
        root.pageCursor = root.nextCursor;
        root.pageNumber += 1;
        root.refresh(false);
    }

    function previousPage() {
        if (!root.cursorStack.length) return;
        root.pageCursor = root.cursorStack[root.cursorStack.length - 1];
        root.cursorStack = root.cursorStack.slice(0, root.cursorStack.length - 1);
        root.pageNumber = Math.max(0, root.pageNumber - 1);
        root.refresh(false);
    }

    function lookupFocusedJob() {
        if (!root.focusedJobId || root.focusedLoading)
            return;
        var present = root.jobs.some(function(job) { return String(job.id || '') === root.focusedJobId; });
        if (present) {
            root.focusedJob = root.jobs.find(function(job) { return String(job.id || '') === root.focusedJobId; }) || root.focusedJob;
            return;
        }
        root.focusedLoading = true;
        Backend.send('collection-job-status', {id: root.focusedJobId}, function(result, ok, message) {
            root.focusedLoading = false;
            if (ok) root.focusedJob = result;
            else root.error = message || 'Focused collection job unavailable';
        }, true);
    }

    function addSource() {
        if (!root.location)
            return ;
        root.busy = true;
        Backend.send('collection-add', {
            'type': root.kind,
            'path': root.location,
            'cadence_hours': root.cadenceHours,
            'auto_briefing': root.autoBriefing && Backend.prefs.localModel === true,
            'goal_ids': root.goalId?[root.goalId]:[]
        }, (result, ok, message) => {
            if (!root) return;
            root.busy = false;
            if (!ok) {
                root.error = message || 'Could not add source';
                return ;
            }
            root.error = '';
            root.location = '';
            root.refresh(true);
        }, true);
    }

    function checkNow(id) {
        root.busy = true;
        Backend.send('collection-check', id ? {'id': id} : {}, (result, ok, message) => {
            if (!root) return;
            root.busy = false;
            if (!ok) {
                root.error = message || 'Could not queue source check';
                return ;
            }
            root.error = '';
            var jobIds = result.job_ids || [];
            if (jobIds.length) root.focusedJobId = String(jobIds[0]);
            root.refresh(false);
        }, true);
    }

    function removeSource(id) {
        Backend.send('collection-remove', {'id': id}, (result, ok, message) => {
            if (!root) return;
            if (!ok)
                root.error = message || 'Could not remove source';
            else
                root.refresh(true);
        }, true);
    }

    function poll() {
        if (!Backend.available || !Backend.restored)
            return ;
        Backend.send('collection-poll', {}, (result, ok, message) => {
            if (!root) return;
            if (!ok)
                root.error = message || 'Collection worker unavailable';
            else
                root.refresh(false);
        }, true);
    }

    Connections {
        target: Backend
        function onPrivacyApplied() { root.refresh(false); }
    }
    Component.onCompleted: { root.refresh(true); Backend.refreshModelChosen() }
    onVisibleChanged: if (visible) { root.refresh(true); Backend.refreshModelChosen() }

    Timer {
        interval: 750
        repeat: true
        running: root.visible && root.jobs.some(job => job.state === "queued" || job.state === "running" || (job.state === "completed" && !job.acked))
        onTriggered: root.poll()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        spacing: 10

        RowLayout {
            Layout.fillWidth: true
            LabelText {
                Layout.fillWidth: true
                color: Theme.accent
                font.pixelSize: 20
                font.weight: Font.Medium
                text: 'Sources'
            }
        }

        LabelText {
            Layout.fillWidth: true
            color: Theme.muted
            font.pixelSize: 12
            text: root.schedulesEnabled ? 'Scheduled collection enabled' : 'Scheduled collection paused'
            wrapMode: Text.Wrap
        }

        RowLayout {
            Layout.fillWidth: true
            Choice {
                Layout.preferredWidth: 150
                model: [{id:'youtube-channel',title:'YouTube channel'},{id:'youtube-video',title:'YouTube video'},{id:'article',title:'Article'},{id:'feed',title:'News feed'},{id:'transcript',title:'Transcript file'},{id:'browser-history',title:'Browser history'}]
                textRole:'title'
                currentIndex: Math.max(0,model.findIndex(i=>i.id===root.kind))
                onActivated: root.kind = model[currentIndex].id
                Accessible.name: 'Source type'
            }
            Field {
                Layout.fillWidth: true
                placeholderText: root.kind==='youtube-channel'?'https://youtube.com/@channel':root.kind==='youtube-video'?'Paste a video link':['article','feed'].includes(root.kind)?'Paste a source URL':'Choose a local source file'
                text: root.location
                onTextEdited: root.location = text
                onAccepted: root.addSource()
                Accessible.name: 'Source path or URL'
            }
            SButton { text:'Browse'; visible:['transcript','browser-history','feed'].includes(root.kind); onClicked:sourcePicker.open() }
            Choice {
                Layout.preferredWidth: 100
                model: [12, 24]
                currentIndex: model.indexOf(root.cadenceHours)
                onActivated: root.cadenceHours = Number(currentText)
                Accessible.name: 'Check every (hours)'
            }
            SButton {
                primary: true
                enabled: !!root.location && !root.busy
                text: 'Add source'
                onClicked: root.addSource()
            }
        }

        RowLayout {
            Layout.fillWidth:true
            Loader {active:Backend.prefs.localModel === true;visible:active;sourceComponent:SCheckBox {text:!Backend.modelChosen ? 'Waiting for your model' : root.schedulesEnabled ? 'Generate a local briefing after each collection' : 'Automatic briefings paused — generate from Learning';enabled:root.schedulesEnabled;checked:root.autoBriefing;onToggled:root.autoBriefing=checked}}
            Choice {Layout.fillWidth:true;model:[{id:'',title:'Consider all active goals'}].concat(Backend.goals||[]);textRole:'title';onActivated:root.goalId=model[currentIndex].id}
        }
        LabelText {
            Layout.fillWidth: true
            color: Theme.dangerText
            text: root.error
            visible: !!root.error
            wrapMode: Text.Wrap
        }

        RowLayout {
            Layout.fillWidth: true
            LabelText { Layout.fillWidth: true; color: Theme.muted; text: 'Page ' + (root.pageNumber + 1); elide: Text.ElideRight }
            SButton { quiet: true; text: 'Previous'; enabled: root.cursorStack.length > 0 && !root.busy; onClicked: root.previousPage() }
            SButton { quiet: true; text: 'Next'; enabled: !!root.nextCursor && !root.busy; onClicked: root.nextPage() }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 10

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                color: Theme.base
                border.color: Theme.line
                radius: Theme.cornerRadius

                ListView {
                    id: sourceList
                    anchors.fill: parent
                    anchors.margins: 6
                    clip: true
                    model: root.sources
                    spacing: 3
                    ScrollBar.vertical: SScrollBar {}

                    delegate: ItemDelegate {
                        required property var modelData
                        clip: true
                        width: sourceList.width
                        height: 108
                        Accessible.name: modelData.kind + ', ' + modelData.location
                        background: Rectangle { color: parent.hovered ? Theme.hover : 'transparent'; radius: Theme.cornerRadius }
                        contentItem: Column {
                            anchors.fill: parent
                            anchors.margins: 8
                            spacing: 2
                            LabelText { width: parent.width; color: Theme.text; text: modelData.kind + (root.schedulesEnabled ? ' · every ' + modelData.cadence_hours + 'h' : ' · schedule paused'); maximumLineCount: 1; wrapMode: Text.NoWrap; elide: Text.ElideRight }
                            LabelText { width: parent.width; color: Theme.muted; text: modelData.location; maximumLineCount: 1; wrapMode: Text.NoWrap; elide: Text.ElideMiddle }
                            LabelText { width: parent.width; color: modelData.last_error ? Theme.danger : Theme.muted; font.pixelSize: 12; text: modelData.last_error || (modelData.last_success ? 'Last success: ' + modelData.last_success : 'Not checked yet'); maximumLineCount: 1; wrapMode: Text.NoWrap; elide: Text.ElideRight }
                        }
                        Row {anchors.right:parent.right;anchors.bottom:parent.bottom;anchors.margins:6;spacing:6
                            SButton {text:'Collect source';quiet:true;enabled:!root.busy;onClicked:root.checkNow(modelData.id)}
                            SButton {text:'Stop following';quiet:true;onClicked:root.removeSource(modelData.id)}
                        }

                    }
                }

                LabelText { anchors.centerIn: parent; color: Theme.muted; text: root.busy ? 'Checking…' : 'No configured sources'; visible: !root.sources.length }
            }

            Rectangle {
                Layout.preferredWidth: Math.min(370,root.width*.40)
                Layout.fillHeight: true
                color: Theme.base
                border.color: Theme.line
                radius: Theme.cornerRadius
                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 12
                    spacing: 8
                    LabelText { Layout.fillWidth: true; color: Theme.accent; font.pixelSize: 13; text: 'Queue and coverage' }
                    LabelText {
                        Layout.fillWidth: true
                        visible: !!root.focusedJob && !root.jobs.some(function(job) { return String(job.id || '') === root.focusedJobId; })
                        color: Theme.accent
                        text: root.focusedJob ? 'Focused job outside this page: ' + String(root.focusedJob.state || 'unknown') + ' · ' + String(root.focusedJob.id || root.focusedJobId) : ''
                        wrapMode: Text.Wrap
                    }
                    ListView {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: root.jobs
                        spacing: 4
                        delegate: LabelText {
                            required property var modelData
                            width: parent ? parent.width : 300
                            color: modelData.state === 'failed' ? Theme.danger : Theme.text
                            font.pixelSize: 12
                            text: modelData.state + ' · ' + ((root.sources.find(s=>s.id===modelData.source_id)||{}).location||modelData.source_id) + (modelData.child_sources?' · '+modelData.child_sources.length+' video sources':'') + (modelData.error ? '\n' + modelData.error : '')
                            wrapMode: Text.Wrap
                        }
                    }
                }
            }
        }
    }
    FileDialog {id:sourcePicker;title:'Choose a source';onAccepted:{let path=String(selectedFile).replace(/^file:\/\//,'');try{root.location=decodeURIComponent(path);}catch(e){root.location=path;}}}
}
