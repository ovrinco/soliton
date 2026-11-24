import QtQuick

// A status badge of the skill manager: accent wash with an accent hairline,
// or the danger tone for a link that needs attention.
Rectangle {
    id: badge
    property string text: ''
    property bool warning: false
    property bool quiet: false
    readonly property color tone: warning ? Theme.dangerText : quiet ? Theme.muted : Theme.accentText
    implicitWidth: caption.implicitWidth + 16
    implicitHeight: caption.implicitHeight + 8
    radius: Theme.chromeRadius
    color: quiet ? 'transparent' : warning ? Qt.tint(Theme.base, Qt.alpha(Theme.danger, .14)) : Theme.wash
    border.width: 1
    border.color: warning ? Theme.danger : quiet ? Theme.line : Theme.accent
    Accessible.role: Accessible.StaticText
    Accessible.name: text
    Text {
        id: caption
        anchors.centerIn: parent
        text: badge.text
        textFormat: Text.PlainText
        color: badge.tone
        font.family: Theme.mono
        font.pixelSize: 12
        font.weight: Font.DemiBold
    }
}
