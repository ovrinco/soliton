import QtQuick
Text {
    property string suffix: ''
    font.family: Theme.font
    font.pixelSize: 15
    font.weight: Font.DemiBold
    font.letterSpacing: 0
    color: Theme.text
    textFormat: Text.PlainText
    elide: Text.ElideRight
}
