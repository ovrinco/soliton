import QtQuick
import QtQuick.Layouts

Item {
    id: root
    property string title: ''
    property string context: ''
    implicitHeight: 66
    implicitWidth: 300
    RowLayout {
        anchors.fill: parent
        spacing: 16
        LabelText {
            Layout.fillWidth: true
            text: root.title
            font.pixelSize: Theme.pageTitleSize
            font.weight: Font.DemiBold
            font.letterSpacing: -0.8
            elide: Text.ElideRight
        }
        LabelText {
            visible: root.width >= 680 && root.context.length > 0
            text: root.context
            font.pixelSize: 13
            color: Theme.muted
        }
    }
}
