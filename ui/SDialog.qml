import QtQuick
import QtQuick.Controls

Dialog {
    id: control
    // Dirty consumers set dirty=true and resolve dirtyDismissalRequested with
    // their existing Save / Discard / Cancel flow. Clean dialogs dismiss from
    // outside click or Escape through the same method.
    property bool dirty: false
    signal dirtyDismissalRequested()

    // Set when the dialog's edits are kept elsewhere (a stored draft), so
    // closing it loses nothing.
    property bool keepsDraft: false

    // Private mode turning on closes a dialog with nothing to lose. A dialog
    // holding unsaved edits stays open with them; if its item is hidden, the
    // dialog's owner keeps the edits and closes it.
    Connections {
        target: Backend
        function onPrivateModeChanged() {
            if (Backend.privateMode && control.visible && (!control.dirty || control.keepsDraft)) control.close();
        }
    }

    function requestDismiss() {
        if (dirty) {
            dirtyDismissalRequested();
            return;
        }
        close();
    }

    // The privacy key reaches past an open dialog, as it does past a menu.
    Shortcut {
        sequence: 'Ctrl+Shift+P'
        enabled: control.visible
        onActivated: Backend.togglePrivateMode()
        onActivatedAmbiguously: Backend.togglePrivateMode()
    }

    closePolicy: Popup.NoAutoClose
    focus: true
    Shortcut {
        enabled: control.visible && control.activeFocus
        sequences: [StandardKey.Cancel]
        context: Qt.WindowShortcut
        onActivated: control.requestDismiss()
    }
    Overlay.modal: Rectangle {
        color: Theme.scrim
    }
    // Observe pointer presses without adding a hit-test surface or taking
    // an exclusive grab. Popup items retain Qt's normal overlay stacking.
    function outsideTopmostPopup(position) {
        var overlay = control.Overlay.overlay;
        var popupItem = control.contentItem.parent;
        var ownIndex = overlay.children.indexOf(popupItem);
        var local = overlay.mapToItem(popupItem, position.x, position.y);
        if (local.x >= 0 && local.y >= 0 && local.x < popupItem.width && local.y < popupItem.height)
            return false;
        for (var i = 0; i < overlay.children.length; ++i) {
            var item = overlay.children[i];
            if (item === popupItem || item.solitonDismissObserver === true || !item.visible) continue;
            // A higher overlay item owns the topmost dismissal. Do not send
            // its outside press to an underlying editor's discard guard.
            if (item.z > popupItem.z || (item.z === popupItem.z && i > ownIndex))
                return false;
        }
        return true;
    }
    // The header's privacy control acts on the same click that closes the
    // dialog, as it does past the palette: one control that always works.
    function findNamed(item, name) {
        if (!item) return null;
        if (item.objectName === name) return item;
        for (var i = 0; i < item.children.length; i++) {
            var found = findNamed(item.children[i], name);
            if (found) return found;
        }
        return null;
    }
    property Item headerPrivate: null
    function onPrivateControl(position) {
        var overlay = control.Overlay.overlay;
        if (!headerPrivate && overlay && overlay.parent)
            headerPrivate = findNamed(overlay.parent, 'soliton-private');
        if (!headerPrivate || !headerPrivate.visible) return false;
        return headerPrivate.contains(overlay.mapToItem(headerPrivate, position.x, position.y));
    }
    property Item dismissalObserver: Item {
        property bool solitonDismissObserver: true
        // A closed observer must leave the overlay entirely. Qt keeps an
        // overlay with ordinary child items alive even when those children
        // are hidden, which otherwise intercepts unrelated page clicks.
        parent: control.visible ? control.Overlay.overlay : null
        anchors.fill: parent
        z: 100000
        visible: control.visible
        // This item accepts no mouse events. The handler observes passively
        // and never takes an exclusive grab from dialog/child controls.
        PointHandler {
            property bool pressedOutside: false
            acceptedButtons: Qt.LeftButton
            onActiveChanged: {
                if (active) {
                    pressedOutside = control.outsideTopmostPopup(point.position);
                } else {
                    // Decide ownership on press, then dismiss on release. A
                    // closing top dialog cannot expose the same click to the
                    // observer beneath it.
                    var dismiss = pressedOutside;
                    var onPrivate = dismiss && control.onPrivateControl(point.position);
                    pressedOutside = false;
                    if (dismiss && control.visible) control.requestDismiss();
                    if (onPrivate) Backend.togglePrivateMode();
                }
            }
            onCanceled: pressedOutside = false
        }
    }
    palette.base: Theme.panel
    palette.text: Theme.text
    palette.window: Theme.base
    palette.windowText: Theme.text
    palette.button: Theme.raised
    palette.buttonText: Theme.text
    palette.highlight: Theme.accent
    palette.highlightedText: Theme.accentForeground
    font.family: Theme.font
    font.pixelSize: Theme.fontSizeNormal
    background: Rectangle { color: Theme.panel; border.color: Theme.edge }
    // The title row carries the dialog's X: the same dismissal as Escape and
    // an outside click, so a dirty dialog still asks before it closes.
    header: Item {
        visible: control.title.length > 0
        implicitHeight: Math.max(heading.implicitHeight, closeButton.height + 2 * Theme.space8)
        LabelText {
            id: heading
            anchors.left: parent.left
            anchors.right: closeButton.left
            anchors.top: parent.top
            text: control.title
            padding: Theme.space16
            rightPadding: Theme.space8
            font.pixelSize: Theme.sectionTitleSize
            font.weight: Font.Medium
            color: Theme.text
            wrapMode: Text.Wrap
        }
        SButton {
            id: closeButton
            objectName: 'dialog-close'
            anchors.right: parent.right
            anchors.rightMargin: Theme.space8
            anchors.top: parent.top
            anchors.topMargin: Theme.space8
            iconName: 'close'
            accessibleLabel: 'Close'
            quiet: true
            onClicked: control.requestDismiss()
        }
    }
}
