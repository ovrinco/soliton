import QtQuick
import QtQuick.Layouts

Rectangle {
    id: root
    property string title: ''
    property string owner: ''
    property string detail: ''
    color: Theme.base
    ColumnLayout {
        anchors.fill: parent; anchors.margins: Theme.pageInset; spacing: 12
        SectionLabel { text: root.title }
        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.line }
        StateView {
            Layout.fillWidth: true; Layout.fillHeight: true
            title: 'Owner unavailable'
            detail: root.detail
            error: true
        }
        LabelText {
            Layout.fillWidth: true; text: root.owner; visible: !!root.owner
            font.family: Theme.mono; font.pixelSize: Theme.fontSizeSecondary
            color: Theme.muted; wrapMode: Text.WrapAnywhere; elide: Text.ElideNone
        }
    }
}
