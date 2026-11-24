pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts

// The Tasks page's saved views: one row per view, click to apply, Remove to delete.
Column {
    id: savedViews
    property var views: []
    property string activeName: ''
    signal applyRequested(var view)
    signal removeRequested(string name)
    spacing: 12

    LabelText { visible: savedViews.views.length === 0; text: 'No saved views yet'; color: Theme.muted }
    Repeater {
        model: savedViews.views
        delegate: RowLayout {
            id: row
            required property var modelData
            readonly property bool active: row.modelData.name === savedViews.activeName
            width: savedViews.width
            spacing: 8
            SButton {
                objectName: 'tasks-saved-view-' + row.modelData.name
                Layout.fillWidth: true
                text: row.modelData.name + (row.active ? ' ✓' : '')
                quiet: !row.active
                accessibleLabel: 'Apply saved view ' + row.modelData.name
                onClicked: savedViews.applyRequested(row.modelData)
            }
            SButton {
                objectName: 'tasks-remove-view-' + row.modelData.name
                text: 'Remove'
                quiet: true
                accessibleLabel: 'Remove saved view ' + row.modelData.name
                onClicked: savedViews.removeRequested(row.modelData.name)
            }
        }
    }
}
