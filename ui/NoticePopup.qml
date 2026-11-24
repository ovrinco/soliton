import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Popup is not a layout item; safe inside ColumnLayout and StackLayout routes.
SPopup {
    id: notice
    required property Item scope
    property string text: ''
    property bool error: true
    property string dismissedText: ''
    onTextChanged: dismissedText = ''
    onDismissed: dismissedText = text
    signal dismissed()
    parent: Overlay.overlay
    visible: scope.visible && text.length > 0 && text !== dismissedText
    modal: false; dim: false; focus: false; padding: 12
    closePolicy: Popup.NoAutoClose
    width: Math.max(200, Math.min(520, parent ? parent.width - 32 : 520))
    x: parent ? Math.max(16, parent.width - width - 16) : 0
    y: parent ? Math.max(16, parent.height - height - 16) : 0
    background: Rectangle { color: Theme.panel; border.color: notice.error ? Theme.dangerText : Theme.edge; Rectangle { width: 3; height: parent.height; color: notice.error ? Theme.dangerText : Theme.accent } }
    contentItem: RowLayout {
        spacing: 12
        implicitHeight: Math.max(34, Math.min(240, noticeText.implicitHeight))
        Flickable {
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            contentWidth: width; contentHeight: noticeText.implicitHeight
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: SScrollBar { }
            TextEdit { id: noticeText; width: parent.width; text: notice.text; readOnly: true; selectByMouse: true; wrapMode: TextEdit.Wrap; textFormat: TextEdit.PlainText; color: notice.error ? Theme.dangerText : Theme.text; font.family: Theme.font; font.pixelSize: 14; Accessible.role: Accessible.AlertMessage }
        }
        SButton { text: ''; iconName: 'close'; quiet: true; accessibleLabel: 'Dismiss message'; onClicked: notice.dismissed() }
    }
}
