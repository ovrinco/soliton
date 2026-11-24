import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The one three-dot menu: opened at its trigger, closed by Escape or a click
// outside. Callers put MenuAction rows in it and call openAt(button).
SPopup {
    id: menu
    default property alias actions: column.data
    parent: Overlay.overlay
    width: 228
    padding: 6
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    function openAt(item) {
        if (item) trigger = item;
        open();
    }
    contentItem: ColumnLayout {
        id: column
        spacing: 2
    }
}
