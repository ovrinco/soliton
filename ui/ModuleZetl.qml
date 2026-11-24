import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts

Item {
    id: root
    property var moduleInfo: null
    property var jobs: []
    property var selectedJob: null
    property string sourcePath: ""
    property int actionIndex: 0
    property int ownedRunIndex: 0
    readonly property var actions: ["inspect", "validate", "run", "verify", "resume"]
    signal runRequested(string action, var args)
    signal inspectRequested(string id)

    function pathFromUrl(url) {
        var value = String(url || "")
        if (value.indexOf("file://") === 0)
            value = value.slice(7)
        try { return decodeURIComponent(value) } catch (error) { return value }
    }
    function actionName() { return root.actions[root.actionIndex] }
    function moduleJobs() {
        var result = []
        for (var i = 0; i < root.jobs.length; ++i)
            if (root.jobs[i].module === "zetl") result.push(root.jobs[i])
        return result
    }
    function ownedRuns() {
        return root.moduleJobs().filter(function(job) {
            return job.evidence && (job.evidence.manifest || job.evidence.config)
        })
    }
    function selectedOwnedRun() {
        var runs = root.ownedRuns()
        return runs.length && root.ownedRunIndex < runs.length ? runs[root.ownedRunIndex] : null
    }
    function selectedPath(kind) {
        var run = root.selectedOwnedRun()
        return run && run.evidence ? String(run.evidence[kind] || "") : ""
    }
    function selectedOutput() { return root.selectedJob && root.selectedJob.evidence ? String(root.selectedJob.evidence.stdout || "") : "" }
    function parsedOutput() {
        var text = root.selectedOutput()
        if (!text.length) return null
        try { return JSON.parse(text) } catch (error) { return null }
    }
    function profileRows() {
        var value = root.parsedOutput()
        if (!value) return []
        var rows = value.columns || value.fields || value.profile || value.schema || value.rows
        if (Array.isArray(rows)) return rows
        if (rows && typeof rows === "object") return Object.keys(rows).map(function(key) { return {name: key, value: rows[key]} })
        return []
    }
    function metric(name, fallback) {
        var value = root.parsedOutput()
        if (!value || value[name] === undefined || value[name] === null) return fallback
        return String(value[name])
    }

    FileDialog {
        id: inputFile
        title: "Choose delimited data"
        fileMode: FileDialog.OpenFile
        nameFilters: ["Delimited data (*.csv *.tsv *.json)", "All files (*)"]
        onAccepted: root.sourcePath = root.pathFromUrl(selectedFile)
    }
    FolderDialog {
        id: inputFolder
        title: "Choose a data folder"
        onAccepted: root.sourcePath = root.pathFromUrl(selectedFolder)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 10
        LabelText {
            Layout.fillWidth: true
            text: "Profile and validate a bounded input, run the fixed pipeline, then verify or resume an owned run. Completion manifests and engine receipts expose rows in/out, reject counts, conservation, checkpoints, and artifacts."
            color: Theme.muted
            wrapMode: Text.Wrap
        }
        RowLayout {
            Layout.fillWidth: true
            LabelText { text: "Pipeline step"; color: Theme.muted }
            Choice {
                id: actionChoice
                Layout.preferredWidth: 180
                model: root.actions
                currentIndex: root.actionIndex
                onActivated: root.actionIndex = currentIndex
                Accessible.name: "Z-ETL pipeline step"
            }
            Item { Layout.fillWidth: true }
            LabelText { text: root.actionName() === "verify" || root.actionName() === "resume" ? "Choose an owned run below" : "Choose new input"; color: Theme.muted }
        }
        Rectangle {
            Layout.fillWidth: true
            visible: root.actionIndex < 3
            implicitHeight: inputForm.implicitHeight + 28
            color: Theme.panel
            border.color: Theme.line
            radius: Theme.cornerRadius
            ColumnLayout {
                id: inputForm
                anchors.fill: parent
                anchors.margins: 14
                spacing: 8
                LabelText { text: root.actionName().charAt(0).toUpperCase() + root.actionName().slice(1) + " input"; font.weight: Font.DemiBold }
                RowLayout {
                    Layout.fillWidth: true
                    Field { Layout.fillWidth: true; text: root.sourcePath; placeholderText: "Choose a data file or folder"; readOnly: true }
                    SButton { text: "Browse file"; onClicked: inputFile.open() }
                    SButton { text: "Browse folder"; quiet: true; onClicked: inputFolder.open() }
                    SButton {
                        text: root.actionName().charAt(0).toUpperCase() + root.actionName().slice(1)
                        primary: true
                        enabled: root.sourcePath.length > 0 && root.moduleInfo && root.moduleInfo.state === "available"
                        onClicked: root.runRequested(root.actionName(), {source_path: root.sourcePath})
                    }
                }
            }
        }
        Rectangle {
            Layout.fillWidth: true
            visible: root.actionIndex >= 3
            implicitHeight: ownedForm.implicitHeight + 28
            color: Theme.panel
            border.color: Theme.line
            radius: Theme.cornerRadius
            ColumnLayout {
                id: ownedForm
                anchors.fill: parent
                anchors.margins: 14
                spacing: 8
                LabelText { text: root.actionName() === "verify" ? "Verify an owned manifest" : "Resume an owned pipeline"; font.weight: Font.DemiBold }
                LabelText { Layout.fillWidth: true; text: "Paths are taken from durable module receipts. This control never accepts a hand-entered manifest or config path."; color: Theme.muted; wrapMode: Text.Wrap }
                RowLayout {
                    Layout.fillWidth: true
                    Choice {
                        id: ownedChoice
                        Layout.fillWidth: true
                        model: root.ownedRuns().map(function(job) { return String(job.action || "run") + " · " + String(job.state || "unknown") + " · " + String(job.id || "") })
                        currentIndex: Math.min(root.ownedRunIndex, Math.max(0, model.length - 1))
                        onActivated: root.ownedRunIndex = currentIndex
                        Accessible.name: "Owned Z-ETL run"
                    }
                    SButton {
                        text: root.actionName() === "verify" ? "Verify manifest" : "Resume run"
                        primary: true
                        enabled: root.selectedOwnedRun() !== null && root.moduleInfo && root.moduleInfo.state === "available" && (root.actionName() === "verify" ? root.selectedPath("manifest") : root.selectedPath("config")) !== ""
                        onClicked: {
                            if (root.actionName() === "verify")
                                root.runRequested("verify", {manifest_path: root.selectedPath("manifest")})
                            else
                                root.runRequested("resume", {config_path: root.selectedPath("config")})
                        }
                    }
                }
                LabelText {
                    Layout.fillWidth: true
                    text: root.actionName() === "verify" ? (root.selectedPath("manifest") || "No owned manifest available") : (root.selectedPath("config") || "No owned resumable config available")
                    color: root.selectedOwnedRun() ? Theme.muted : Theme.danger
                    elide: Text.ElideMiddle
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: root.profileRows().length > 0 || root.selectedJob
            LabelText { text: "Run evidence"; font.weight: Font.DemiBold }
            LabelText { text: "Rows in " + root.metric("rows_in", root.metric("input_rows", "—")) + " · rows out " + root.metric("rows_out", root.metric("output_rows", "—")); color: Theme.muted }
            LabelText { text: "Rejected " + root.metric("reject_count", root.metric("rejected", "—")); color: Theme.muted }
            LabelText { text: "Conservation " + root.metric("conservation", "—"); color: Theme.muted }
        }
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: Theme.panel
            border.color: Theme.line
            radius: Theme.cornerRadius
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 14
                spacing: 8
                RowLayout {
                    Layout.fillWidth: true
                    LabelText { text: root.selectedJob ? (String(root.selectedJob.action || "run") + " · " + String(root.selectedJob.state || "unknown")) : "Pipeline receipt"; font.weight: Font.DemiBold }
                    Item { Layout.fillWidth: true }
                    LabelText { text: root.selectedJob && root.selectedJob.evidence && root.selectedJob.evidence.manifest ? "Manifest retained" : "No manifest yet"; color: Theme.muted }
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: 6
                    Repeater {
                        model: [
                            {label: "Inspect", action: "inspect"},
                            {label: "Validate", action: "validate"},
                            {label: "Run", action: "run"},
                            {label: "Verify", action: "verify"},
                            {label: "Resume", action: "resume"}
                        ]
                        delegate: Rectangle {
                            required property var modelData
                            width: 92
                            height: 38
                            color: root.actionName() === modelData.action ? Theme.selection : Theme.base
                            border.color: Theme.line
                            radius: Theme.cornerRadius
                            LabelText { anchors.centerIn: parent; text: modelData.label; color: Theme.text }
                        }
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    visible: root.selectedJob && root.selectedJob.evidence
                    LabelText { Layout.fillWidth: true; text: root.selectedJob && root.selectedJob.evidence && root.selectedJob.evidence.output ? "Output: " + root.selectedJob.evidence.output : "Output: —"; color: Theme.muted; elide: Text.ElideMiddle }
                    LabelText { Layout.fillWidth: true; text: root.selectedJob && root.selectedJob.evidence && root.selectedJob.evidence.checkpoint ? "Checkpoint: " + root.selectedJob.evidence.checkpoint : "Checkpoint: —"; color: Theme.muted; elide: Text.ElideMiddle }
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    visible: root.profileRows().length > 0
                    clip: true
                    spacing: 4
                    model: root.profileRows()
                    delegate: Rectangle {
                        required property var modelData
                        width: ListView.view ? ListView.view.width : 400
                        height: 32
                        color: Theme.base
                        border.color: Theme.line
                        RowLayout {
                            anchors.fill: parent
                            anchors.margins: 7
                            LabelText { Layout.fillWidth: true; text: String(modelData.name || modelData.column || modelData.field || modelData.key || "field"); font.weight: Font.DemiBold; elide: Text.ElideRight }
                            LabelText { Layout.preferredWidth: 220; text: String(modelData.type || modelData.value || modelData.count || "—"); color: Theme.muted; elide: Text.ElideRight }
                        }
                    }
                }
                ScrollView { contentWidth: availableWidth;
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    visible: root.profileRows().length === 0
                    clip: true
                    STextArea { width: parent.width; readOnly: true; wrapMode: TextArea.Wrap; text: root.selectedOutput().length ? root.selectedOutput() : "Run inspect, validation, or a pipeline step to see exact engine evidence. No progress, reject count, or conservation result is inferred here."; color: Theme.text; font.family: Theme.mono; font.pixelSize: 12; background: null }
                }
            }
        }
    }
}
