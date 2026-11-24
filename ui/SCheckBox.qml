import QtQuick
import QtQuick.Controls as Controls

Controls.CheckBox {
    id: control
    implicitHeight: Math.max(Theme.controlHeight, contentItem.implicitHeight + topPadding + bottomPadding)
    implicitWidth: contentItem.implicitWidth + leftPadding + rightPadding
    padding: 6
    spacing: Theme.controlSpacing
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    font.family: Theme.font
    font.pixelSize: Theme.fontSizeNormal
    indicator: Rectangle {
        implicitWidth: Theme.indicatorSize
        implicitHeight: Theme.indicatorSize
        x: control.mirrored ? control.width - width - control.rightPadding : control.leftPadding
        y: (control.height - height) / 2
        radius: Theme.chromeRadius
        color: control.checkState === Qt.Unchecked ? Theme.base : Theme.accent
        border.width: control.activeFocus ? 2 : 1
        border.color: !control.enabled ? Theme.line : control.activeFocus || control.hovered ? Theme.accent : Theme.muted
        opacity: control.enabled ? 1 : 0.55
        Text {
            anchors.centerIn: parent
            text: control.checkState === Qt.PartiallyChecked ? '−' : control.checked ? '✓' : ''
            font.pixelSize: Theme.fontSizeNormal - 1
            font.bold: true
            color: Theme.accentForeground
        }
    }
    contentItem: Text {
        text: control.text
        font: control.font
        color: control.enabled ? Theme.buttonSecondaryText : Theme.buttonDisabledText
        verticalAlignment: Text.AlignVCenter
        horizontalAlignment: Text.AlignLeft
        wrapMode: Text.Wrap
        leftPadding: control.mirrored ? 0 : control.indicator.width + control.spacing
        rightPadding: control.mirrored ? control.indicator.width + control.spacing : 0
        elide: Text.ElideRight
    }
}
