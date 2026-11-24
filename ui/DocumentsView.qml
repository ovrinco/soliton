import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Workplane.js" as Workplane

Item {
    id: root
    property string kind: 'note'
    property string strategyTab: 'Ideation'
    property bool embedded: false
    property var documents: documentsModel.model
    readonly property int rowHeight: 44
    // A short plain excerpt: the body's first sentences, without its title,
    // its short label lines or markdown markers.
    function excerpt(record) {
        var title = String(record.title || '').trim();
        var lines = String(record.body || '').slice(0, 1200).split('\n')
            .map(line => line.replace(/^\s*(#+|>|[-*+]|\d+[.)])\s+/, '').replace(/[*_`]+/g, '').trim())
            .filter(line => line.length > 0 && line !== title);
        var prose = lines.filter(line => line.length >= 40);
        return (prose.length ? prose : lines).join(' ');
    }
    RetainedModel { id: documentsModel; active: root.visible; sourceModel: Backend.records.filter(r => r.kind === kind && Backend.matches(r)).sort((a, b) => b.updated_at - a.updated_at) }
    ColumnLayout {
        anchors.fill: parent
        spacing: 8
        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 40
            SSearchField { Layout.fillWidth: true; Layout.minimumWidth: 40; frameless: true; scopeName: root.kind === 'strategy' ? 'strategies' : 'notes'; text: Backend.search; onTextEdited: { Backend.search=text; Backend.remember(); } }
            SButton { iconName: 'filter'; quiet: true; accessibleLabel: 'Filter these documents'; onClicked: documentFilters.open() }
            SButton {
                iconName: 'plus'
                text: root.kind === 'learning' ? 'Add study note' : root.kind === 'strategy' ? 'Add strategy' : 'Add note'
                primary: true
                onClicked: Backend.create(root.kind)
            }
        }
        RowLayout {
            visible: root.kind === 'strategy' && !root.embedded
            STabButton {
                text: 'Ideation'
                active: root.strategyTab === 'Ideation'
                onClicked: root.strategyTab = 'Ideation'
            }
            STabButton {
                text: 'Execution'
                active: root.strategyTab === 'Execution'
                onClicked: root.strategyTab = 'Execution'
            }
        }
        ListView {
            id: list
            SCollectionWheel { flickable: list }
            objectName: root.kind + '-list'
            RetainedScroll { flickable: list; positionKey: root.kind + '/list' }
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 0
            model: root.documents
            delegate: Item {
                id: doc
                required property var modelData
                property var work: root.kind === 'strategy' && root.strategyTab === 'Execution'
                    ? Backend.records.filter(r => r.kind === 'task' && (r.links || []).indexOf(modelData.id) >= 0 && !r.hidden) : []
                width: list.width
                height: root.rowHeight + work.length * 24
                DocumentRow {
                    width: parent.width
                    height: root.rowHeight
                    heading: doc.modelData.title || 'Untitled'
                    excerpt: root.excerpt(doc.modelData)
                    date: Qt.formatDate(new Date(doc.modelData.updated_at * 1000), 'MMM d, yyyy')
                    metadata: { var n = (doc.modelData.files || []).length; return n ? n + (n === 1 ? ' source file' : ' source files') : ''; }
                    selected: Backend.selectedId === doc.modelData.id
                    onClicked: Backend.select(doc.modelData.id)
                    onDoubleClicked: { Backend.select(doc.modelData.id); Backend.detailFull = true; }
                    Keys.onReturnPressed: { Backend.select(doc.modelData.id); Backend.detailFull = true; }
                }
                Column {
                    x: 14; y: root.rowHeight; width: parent.width - 28
                    Repeater {
                        model: doc.work
                        LabelText { required property var modelData; width: parent.width; height: 24; text: (modelData.status === 'completed' ? '✓ ' : '○ ') + modelData.title; color: Theme.muted; font.pixelSize: 12 }
                    }
                }
            }
            StateView {
                anchors.fill: parent
                visible: root.documents.length === 0
                title: 'No ' + (root.kind === 'note' ? 'notes' : root.kind === 'learning' ? 'study notes' : 'strategies') + ' in this view'
                detail: Backend.search || Backend.branches.length ? 'Change the current filters to see other documents.' : 'Create a document when there is something to keep.'
            }
            ScrollBar.vertical: SScrollBar {}
        }

    }
    // One compact entry of a document list (notes, study notes, strategy ideas):
    // the title and date on the first line, a short excerpt on the second.
    component DocumentRow: AbstractButton {
        id: row
        padding: 0
        clip: true
        property string heading: ''
        property string excerpt: ''
        property string date: ''
        property string metadata: ''
        property bool selected: false
        implicitHeight: 44
        hoverEnabled: true
        activeFocusOnTab: true
        Accessible.role: Accessible.ListItem
        Accessible.name: heading + (date ? '. ' + date : '') + (metadata ? '. ' + metadata : '') + (excerpt ? '. ' + excerpt : '')
        Accessible.selected: selected
        background: Rectangle {
            color: row.selected ? Theme.wash : row.down ? Theme.selection : row.hovered ? Theme.hover : 'transparent'
            border.width: row.visualFocus ? 2 : 0
            border.color: Theme.accent
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line }
            Rectangle { width: 2; height: parent.height - 12; y: 6; color: Theme.accent; visible: row.selected }
        }
        contentItem: Item {
            GridLayout {
                x: 14; width: parent.width - 28; y: (parent.height - height) / 2
                columns: 2; columnSpacing: 12; rowSpacing: 2
                LabelText { Layout.fillWidth: true; text: Workplane.previewLine(row.heading); wrapMode: Text.NoWrap; maximumLineCount: 1; elide: Text.ElideRight; font.pixelSize: 14; font.weight: Font.Medium }
                LabelText { Layout.alignment: Qt.AlignRight; text: row.date; wrapMode: Text.NoWrap; color: Theme.muted; font.pixelSize: 12 }
                LabelText { Layout.fillWidth: true; text: Workplane.previewLine(row.excerpt); wrapMode: Text.NoWrap; maximumLineCount: 1; elide: Text.ElideRight; color: Theme.muted; font.pixelSize: 12 }
                LabelText { Layout.alignment: Qt.AlignRight; text: row.metadata; visible: text.length > 0; wrapMode: Text.NoWrap; color: Theme.muted; font.pixelSize: 12 }
            }
        }
        ToolTip.visible: hovered && (heading.length > 60 || excerpt.length > 90)
        ToolTip.text: heading + (metadata ? '\n' + metadata : '') + (excerpt ? '\n\n' + Workplane.previewLine(excerpt).slice(0, 280) : '')
        ToolTip.delay: 800
    }
    ScopeFilters { id: documentFilters; scopeName: root.kind === 'strategy' ? 'strategies' : 'notes' }
}
