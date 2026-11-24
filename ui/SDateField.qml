import QtQuick
import QtQuick.Controls

Item {
    id: root
    property string dateValue: ''
    property string contextKey: ''
    property bool resettingContext: false
    function refreshValue() { field.text = displayDate(); field.invalid = false; }
    onContextKeyChanged: {
        resettingContext = true; refreshValue(); picker.close(); field.focus = false;
        Qt.callLater(function() { root.resettingContext = false; });
    }
    property string placeholderText: 'Choose a date'
    property string accessibleName: placeholderText
    property bool showClearButton: true
    signal dateEdited(string isoDate)
    // The owner commits edits, keeping its dateValue binding intact.
    function commit(value) { field.invalid = false; dateEdited(value); field.text = displayDate(); }
    function commitText() {
        var value = field.text.trim();
        if (!value) { commit(''); return; }
        var match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
        var parsed = match ? new Date(Number(match[1]), Number(match[2])-1, Number(match[3])) : Date.fromLocaleString(Qt.locale(), value, 'MMM d, yyyy');
        if (isNaN(parsed.getTime()) || (match && (parsed.getFullYear() !== Number(match[1]) || parsed.getMonth()+1 !== Number(match[2]) || parsed.getDate() !== Number(match[3])))) {
            field.invalid = true; field.errorText = 'Enter a valid date as YYYY-MM-DD or use the calendar.'; return;
        }
        commit(Qt.formatDate(parsed, 'yyyy-MM-dd'));
    }
    onDateValueChanged: refreshValue()
    implicitWidth: Theme.inputWidth + 2 * Theme.innerButtonSize
    implicitHeight: Theme.controlHeight

    function openPicker() { picker.open(); }

    function displayDate() {
        if (!dateValue)
            return '';
        var parts = dateValue.split('-');
        if (parts.length !== 3)
            return dateValue;
        return Qt.formatDate(new Date(Number(parts[0]), Number(parts[1]) - 1, Number(parts[2])), 'MMM d, yyyy');
    }

    Field {
        id: field
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        placeholderText: root.placeholderText
        text: root.displayDate()
        readOnly: false
        objectName: root.objectName ? root.objectName + "-field" : ""
        Accessible.name: root.accessibleName
        rightPadding: Math.max(0, root.width - calendarButton.x) + Theme.space4
        onEditingFinished: if (!root.resettingContext) root.commitText()
        onTextEdited: invalid = false
        Keys.onEscapePressed: { text = root.displayDate(); invalid = false; }
        Keys.onDownPressed: event => { if (event.modifiers & Qt.AltModifier) { picker.open(); event.accepted = true; } }
    }
    // Field-end controls take the neighbouring fields' chevron treatment: the last
    // control sits on the chevron's column, drawn at the chevron's size and colour.
    readonly property real endCenter: width - Theme.controlPadding - Theme.fontSizeSmall / 2
    component EndButton: AbstractButton {
        id: endButton
        property string iconName: ''
        property string accessibleLabel: ''
        readonly property color ink: !enabled ? Theme.buttonDisabledText : hovered || down || visualFocus ? Theme.inputText : Theme.inputPlaceholder
        width: Theme.innerButtonSize
        height: Theme.innerButtonSize
        padding: 0
        hoverEnabled: true
        focusPolicy: Qt.StrongFocus
        contentItem: Item {
            SIcon { anchors.centerIn: parent; visible: !!endButton.iconName; name: endButton.iconName; size: Theme.fontSizeSmall; color: endButton.ink }
            Text { anchors.centerIn: parent; visible: !endButton.iconName; text: endButton.text; font.family: Theme.font; font.pixelSize: Theme.fontSizeSmall; color: endButton.ink }
        }
        background: Rectangle {
            radius: Theme.chromeRadius
            color: endButton.down ? Theme.selection : endButton.hovered ? Theme.hover : Theme.clear
            border.width: endButton.visualFocus ? 2 : 0
            border.color: Theme.accent
        }
        ToolTip.visible: hovered && !!accessibleLabel
        ToolTip.text: accessibleLabel
        ToolTip.delay: 600
        Accessible.role: Accessible.Button
        Accessible.name: accessibleLabel
    }
    EndButton {
        id: clearButton
        objectName: root.objectName ? root.objectName + "-clear" : "date-clear"
        x: Math.round(root.endCenter - width / 2)
        anchors.verticalCenter: parent.verticalCenter
        visible: root.showClearButton && !!root.dateValue
        iconName: 'close'
        accessibleLabel: 'Clear date'
        onClicked: root.commit('')
    }
    EndButton {
        id: calendarButton
        objectName: root.objectName ? root.objectName + "-calendar" : "date-calendar"
        x: Math.round(root.endCenter - (clearButton.visible ? Theme.innerButtonSize : 0) - width / 2)
        anchors.verticalCenter: parent.verticalCenter
        iconName: 'calendar'
        accessibleLabel: 'Choose date'
        onClicked: picker.open()
    }
    SDatePicker {
        id: picker
        trigger: root
        alignRight: true
        selectedDate: root.dateValue
        onDateSelected: value => root.commit(value)
    }
}
