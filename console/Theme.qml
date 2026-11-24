pragma Singleton
import QtQuick
import "../ui" as Planning

QtObject {
    readonly property color accent: Planning.Theme.accent
    readonly property color accentForeground: Planning.Theme.accentForeground
    readonly property int cornerRadius: Planning.Theme.cornerRadius
    readonly property int controlHeight: Planning.Theme.controlHeight
    readonly property int fontSizeNormal: Planning.Theme.fontSizeNormal
    readonly property color base: Planning.Theme.base
    readonly property color danger: Planning.Theme.danger
    readonly property string font: Planning.Theme.font
    readonly property color green: Planning.Theme.green
    readonly property color line: Planning.Theme.line
    readonly property string mono: Planning.Theme.mono
    readonly property color muted: Planning.Theme.muted
    readonly property color panel: Planning.Theme.panel
    readonly property color raised: Planning.Theme.raised
    readonly property color selection: Planning.Theme.selection
    readonly property color text: Planning.Theme.text
    readonly property color edge: Planning.Theme.edge
    readonly property color wash: Planning.Theme.wash
    readonly property color dangerText: Planning.Theme.dangerText
    readonly property color accentText: Planning.Theme.accentText
    readonly property int actionHeight: Planning.Theme.actionHeight
    readonly property int readerSize: Planning.Theme.readerSize
}
