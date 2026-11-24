import QtQuick
import QtQuick.Controls

ToolTip {
    id: hint
    delay: 650
    timeout: 6000
    padding: 8
    contentItem: LabelText {
        text: hint.text
        wrapMode: Text.Wrap
        font.pixelSize: 12
        color: Theme.text
    }
    background: Rectangle {
        color: Theme.raised
        border.color: Theme.line
        radius: Theme.cornerRadius
    }
}
