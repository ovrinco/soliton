pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The Z-ETL panel's host: the bundled helper's bounded modules-* service
// contract, with the panel's own controls in ModuleZetl. It does not
// synthesize run receipts; every job shown is one the service recorded.
Item {
    id: root
    objectName: "module-workspace-zetl"
    readonly property string moduleId: "zetl"
    readonly property var panel: workflow.item
    property var modules: []
    property var jobs: []
    property var selectedJob: null
    property bool loading: false
    property string errorText: ""
    property string noticeText: ""
    property string selectedJobId: ""
    property int requestSerial: 0
    property int pendingRequests: 0
    property var requestGroups: ({list:0, detail:0, action:0})

    function request(command, args, callback) {
        var serial = ++root.requestSerial
        var group = command.endsWith('-list') ? 'list' : command.endsWith('-status') ? 'detail' : 'action'
        root.requestGroups[group] = serial
        root.pendingRequests++
        root.loading = true
        Backend.send(command, args || {}, function(result, ok, error) {
            if (!root) return
            root.pendingRequests--
            root.loading = root.pendingRequests > 0
            if (serial !== root.requestGroups[group]) return
            if (!ok) {
                root.errorText = error || (result && result.error) || "The request failed."
                return
            }
            root.errorText = ""
            callback(result || {})
        })
    }

    function moduleInfo() {
        for (var i = 0; i < root.modules.length; ++i)
            if (root.modules[i].id === root.moduleId)
                return root.modules[i]
        return null
    }

    function refresh() {
        request("modules-list", {}, function(result) {
            root.modules = result.modules || []
            root.jobs = result.jobs || []
            root.selectLatest()
        })
    }

    function selectLatest() {
        var found = null
        for (var i = 0; i < root.jobs.length; ++i) {
            var job = root.jobs[i]
            if (job.module === root.moduleId && (!found || String(job.created_at || "") > String(found.created_at || "")))
                found = job
            if (job.id === root.selectedJobId)
                root.selectedJob = job
        }
        if (root.selectedJob && root.selectedJob.module !== root.moduleId)
            root.selectedJob = null
        if (!root.selectedJob && found) {
            root.selectedJob = found
            root.selectedJobId = found.id
        }
        // modules-list is metadata-only and carries stdout as null; fetch the
        // full receipt for the selected job, keeping the user's selection.
        if (root.selectedJob && (!root.selectedJob.evidence || root.selectedJob.evidence.stdout === undefined || root.selectedJob.evidence.stdout === null))
            root.inspectJob(root.selectedJob.id)
    }

    function syncWorkflow() {
        if (!workflow.item)
            return
        workflow.item.moduleInfo = root.moduleInfo()
        workflow.item.jobs = root.jobs
        workflow.item.selectedJob = root.selectedJob
    }

    function run(action, args) {
        var payload = args || {}
        payload.module = root.moduleId
        payload.action = action
        root.noticeText = ""
        request("modules-run", payload, function(result) {
            root.requestGroups.detail = ++root.requestSerial
            root.selectedJobId = result.id || ""
            root.selectedJob = {"id": result.id || "", "module": root.moduleId, "action": action, "state": result.state || "queued"}
            root.noticeText = "Started " + action + "."
            refreshTimer.restart()
        })
    }

    function inspectJob(id) {
        root.selectedJobId = id
        if (!root.selectedJob || root.selectedJob.id !== id)
            root.selectedJob = root.jobs.find(job => job.id === id) || null
        request("modules-status", {id: id}, function(result) {
            root.selectedJob = result
            root.selectedJobId = id
        })
    }

    function stateColor(state) {
        if (state === "succeeded") return Theme.successText
        if (state === "failed" || state === "timeout") return Theme.dangerText
        if (state === "running" || state === "queued") return Theme.accent
        return Theme.muted
    }

    function moduleJobs() {
        return root.jobs.filter(job => job.module === root.moduleId)
    }

    Timer {
        id: refreshTimer
        interval: 450
        onTriggered: root.refresh()
    }
    Timer {
        interval: 1400
        repeat: true
        running: root.visible
        onTriggered: {
            if (root.selectedJob && (root.selectedJob.state === "queued" || root.selectedJob.state === "running"))
                root.inspectJob(root.selectedJob.id)
        }
    }

    onVisibleChanged: if (root.visible && Backend.available) root.refresh()
    Component.onCompleted: root.refresh()

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            LabelText {
                Layout.fillWidth: true
                text: root.moduleInfo() ? root.moduleInfo().name : "Z-ETL"
                font.pixelSize: Theme.titleSize
                font.weight: Font.DemiBold
            }
            SStateWord {
                readonly property string moduleState: root.moduleInfo() ? String(root.moduleInfo().state || "") : ""
                word: moduleState === "available" ? "Ready" : moduleState === "unavailable" ? "Unavailable" : ""
                tone: moduleState === "available" ? "success" : "danger"
            }
            SButton { text: 'Runs'; Layout.preferredWidth: 90; busy: root.loading; enabled: root.moduleJobs().length > 0 && !root.loading; onClicked: runsPopup.open() }
        }

        Loader {
            id: workflow
            Layout.fillWidth: true
            Layout.fillHeight: true
            source: "ModuleZetl.qml"
            onLoaded: root.syncWorkflow()
        }
        Connections {
            target: workflow.item
            ignoreUnknownSignals: true
            function onRunRequested(action, args) { root.run(action, args) }
            function onInspectRequested(id) { root.inspectJob(id) }
        }
    }

    onModulesChanged: root.syncWorkflow()
    onJobsChanged: root.syncWorkflow()
    onSelectedJobChanged: root.syncWorkflow()
    NoticePopup { scope: root; text: root.errorText || root.noticeText; error: !!root.errorText; onDismissed: { root.errorText = ''; root.noticeText = ''; } }
    SPopup {
        id: runsPopup; parent: Overlay.overlay; anchors.centerIn: parent; modal: true; padding: 16
        width: Math.min(520, parent.width-32); height: Math.min(440, parent.height-32)
        contentItem: ColumnLayout {
            SectionLabel { text: 'Run history' }
            ListView {
                id: runList; Layout.fillWidth: true; Layout.fillHeight: true; clip: true; model: root.moduleJobs()
                delegate: CollectionRow {
                    required property var modelData; width: runList.width
                    heading: String(modelData.action || 'Run'); metadata: String(modelData.state || 'Unknown') + ' · ' + String(modelData.created_at || '')
                    selected: modelData.id === root.selectedJobId; marker: root.stateColor(modelData.state)
                    onClicked: { root.inspectJob(modelData.id); runsPopup.close(); }
                }
                ScrollBar.vertical: SScrollBar { }
            }
            SButton { text: 'Close'; Layout.alignment: Qt.AlignRight; onClicked: runsPopup.close() }
        }
    }
}
