import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: state
    property string title: ''
    property string detail: ''
    property string actionText: ''
    property bool error: false
    property bool pending: false
    signal actionRequested()
    implicitHeight: Math.max(180, message.implicitHeight + 48)
    ColumnLayout {
        id: message
        anchors.centerIn: parent
        width: Math.min(480, Math.max(0, parent.width-40))
        spacing: 12
        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            SIcon { name: state.error ? 'warning' : state.pending ? 'clock' : 'file'; color: state.error ? Theme.dangerText : Theme.muted }
            LabelText { Layout.fillWidth: true; text: state.title; font.pixelSize: 18; font.weight: Font.Medium; wrapMode: Text.Wrap; elide: Text.ElideNone }
        }
        LabelText { Layout.fillWidth: true; visible: !!state.detail; text: state.detail; color: Theme.muted; wrapMode: Text.Wrap; elide: Text.ElideNone; lineHeight: 1.45 }
        SButton { visible: !!state.actionText; text: state.actionText; onClicked: state.actionRequested() }
    }
    Accessible.role: Accessible.StaticText
    Accessible.name: title + (detail ? '. '+detail : '')
}
