import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// A line that edits where it stands. A click opens it; Save appears only
// once the draft differs from the saved text; Enter saves, Escape discards,
// a click elsewhere keeps the draft open. The owner stores the value from
// saved(value) and sets text back from its record.
FocusScope {
    id: root
    property string text: ''
    property string placeholderText: ''
    property int pixelSize: Theme.fontSizeNormal
    property int fontWeight: Font.Normal
    property string accessibleName: placeholderText || text
    property bool editing: false
    readonly property string draft: field.text
    readonly property bool dirty: editing && field.text !== text
    signal saved(string value)
    implicitWidth: Theme.inputWidth + Theme.controlSpacing + saveSlot.implicitWidth
    implicitHeight: Theme.controlHeight
    activeFocusOnTab: true

    function begin() {
        if (!editing) {
            field.text = text;
            editing = true;
        }
        field.forceActiveFocus();
        field.cursorPosition = field.text.length;
    }
    function save() {
        var value = field.text;
        var changed = value !== text;
        editing = false;
        if (changed) saved(value);
    }
    function discard() {
        field.text = text;
        editing = false;
    }

    Keys.onReturnPressed: begin()
    Keys.onEnterPressed: begin()
    Keys.onSpacePressed: begin()

    RowLayout {
        anchors.fill: parent
        spacing: Theme.controlSpacing
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Rectangle {
                anchors.fill: parent
                visible: !root.editing
                radius: Theme.chromeRadius
                color: rest.containsMouse ? Theme.hover : Theme.clear
                border.width: root.activeFocus && !root.editing ? 2 : 0
                border.color: Theme.accent
            }
            LabelText {
                anchors.fill: parent
                visible: !root.editing
                leftPadding: Theme.controlPadding
                rightPadding: Theme.controlPadding
                verticalAlignment: Text.AlignVCenter
                text: root.text || root.placeholderText
                color: root.text ? Theme.text : Theme.inputPlaceholder
                font.pixelSize: root.pixelSize
                font.weight: root.fontWeight
            }
            MouseArea {
                id: rest
                anchors.fill: parent
                visible: !root.editing
                hoverEnabled: true
                cursorShape: Qt.IBeamCursor
                onClicked: root.begin()
            }
            Field {
                id: field
                objectName: root.objectName ? root.objectName + '-field' : ''
                anchors.fill: parent
                visible: root.editing
                placeholderText: root.placeholderText
                font.pixelSize: root.pixelSize
                font.weight: root.fontWeight
                Accessible.name: root.accessibleName
                Keys.onReturnPressed: root.save()
                Keys.onEnterPressed: root.save()
                Keys.onEscapePressed: event => { root.discard(); root.forceActiveFocus(); event.accepted = true; }
                // A click elsewhere keeps a changed draft open; an unchanged
                // one returns to the line.
                onActiveFocusChanged: if (!activeFocus && root.editing && !root.dirty) root.editing = false
            }
        }
        // Save's room is held whether or not it shows, so nothing moves.
        SReserved {
            id: saveSlot
            SButton {
                objectName: root.objectName ? root.objectName + '-save' : ''
                text: 'Save'
                primary: true
                visible: root.dirty
                onClicked: root.save()
            }
        }
    }
    Accessible.role: Accessible.EditableText
    Accessible.name: accessibleName
}
