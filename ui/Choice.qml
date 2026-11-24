import QtQuick
import QtQuick.Controls

ComboBox {
    id: control
    property var labelMap: ({})
    function textFor(value) {
        var label = control.textRole ? value[control.textRole] : value;
        return control.labelMap[label] !== undefined ? control.labelMap[label] : label;
    }
    implicitHeight: Theme.controlHeight
    implicitWidth: Theme.inputWidth
    leftPadding: Theme.controlPadding
    rightPadding: Theme.controlPadding + indicator.width + Theme.space4
    font.family: Theme.font
    font.pixelSize: Theme.fontSizeNormal
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    contentItem: Text {
        text: control.displayText
        color: control.enabled ? Theme.inputText : Theme.buttonDisabledText
        font: control.font
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
    indicator: SIcon {
        x: control.width - Theme.controlPadding - width
        anchors.verticalCenter: control.verticalCenter
        name: 'down'
        size: Theme.fontSizeSmall
        color: Theme.inputPlaceholder
    }
    background: Rectangle {
        radius: Theme.chromeRadius
        color: control.enabled ? (control.hovered ? Theme.hover : Theme.inputFill) : Theme.buttonDisabledFill
        border.width: control.visualFocus ? 2 : 1
        border.color: control.visualFocus ? Theme.accent : Theme.buttonBorder
    }
    popup: Popup {
        y: control.height + 2
        width: Math.max(control.width, Theme.inputWidth)
        padding: Theme.space4
        implicitHeight: Math.min(contentItem.implicitHeight + 2 * Theme.space4, 280)
        background: Rectangle {
            color: Theme.raised
            border.color: Theme.line
            border.width: 1
            radius: Theme.chromeRadius
        }
        contentItem: ListView {
            clip: true
            implicitHeight: contentHeight
            model: control.popup.visible ? control.delegateModel : null
            currentIndex: control.highlightedIndex
            ScrollIndicator.vertical: ScrollIndicator {}
        }
    }
    delegate: ItemDelegate {
        id: row
        required property var modelData
        required property int index
        width: control.popup.width - 2 * Theme.space4
        height: Theme.denseRowHeight
        contentItem: Text {
            text: control.textFor(row.modelData)
            color: Theme.inputText
            font: control.font
            verticalAlignment: Text.AlignVCenter
            leftPadding: Theme.controlPadding
            elide: Text.ElideRight
        }
        background: Rectangle {
            radius: Theme.chromeRadius
            color: row.highlighted ? Theme.selection : Theme.clear
        }
        highlighted: control.highlightedIndex === index
    }
    Accessible.role: Accessible.ComboBox
}
