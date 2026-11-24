import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The habit form in the right pane: Name, Kind, Usually (to break only) and
// Part of. Save and Cancel appear once something has changed.
Item {
    id: form
    objectName: 'habit-form'
    readonly property var original: Backend.habitFormId ? Backend.habit(Backend.habitFormId) : null
    readonly property var kinds: ['To form', 'To break']
    readonly property var places: ['Morning', 'Evening', 'Through the day']
    readonly property var placeKeys: ['morning', 'evening', null]
    property string title: ''
    property int kind: 0
    property string usually: '0'
    property int place: 2
    property bool saving: false
    readonly property bool dirty: !original
        ? title !== '' || kind !== 0 || place !== 2 || (kind === 1 && usually !== '0')
        : title !== original.title || kind !== (original.direction === 'break' ? 1 : 0) || place !== placeIndex(original.bundle)
          || (kind === 1 && Number(usually) !== original.baseline)

    function placeIndex(bundle) { return bundle === 'morning' ? 0 : bundle === 'evening' ? 1 : 2; }
    function reset() {
        var h = original;
        title = h ? h.title : '';
        kind = h && h.direction === 'break' ? 1 : 0;
        usually = h && h.direction === 'break' ? String(h.baseline) : '0';
        place = h ? placeIndex(h.bundle) : 2;
        nameField.text = title; kindChoice.currentIndex = kind; placeChoice.currentIndex = place; usuallyField.text = usually;
        nameField.forceActiveFocus();
    }
    function close() { Backend.habitFormOpen = false; }
    function save() {
        if (!title.trim() || saving) return;
        saving = true;
        Backend.habitSave({
            id: original ? original.id : '',
            title: title.trim(),
            direction: kind === 1 ? 'break' : 'form',
            baseline: kind === 1 ? Math.max(0, Math.floor(Number(usually) || 0)) : 1,
            bundle: placeKeys[place]
        }, ok => { saving = false; if (ok) close(); });
    }

    Connections {
        target: Backend
        function onHabitFormSerialChanged() { form.reset(); }
    }
    Component.onCompleted: reset()
    Keys.onEscapePressed: close()

    ColumnLayout {
        anchors.fill: parent
        spacing: 0
        Item {
            Layout.fillWidth: true; Layout.preferredHeight: 52
            RowLayout {
                anchors.fill: parent; anchors.leftMargin: 24; anchors.rightMargin: 16; spacing: 4
                LabelText { Layout.fillWidth: true; text: 'Habit'; font.pixelSize: 14; font.weight: Font.Medium }
                SButton { iconName: 'close'; accessibleLabel: 'Close'; quiet: true; onClicked: form.close() }
            }
        }
        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.line }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.margins: 24
            spacing: 18

            component Label: LabelText { font.pixelSize: 12; color: Theme.muted }

            ColumnLayout {
                Layout.fillWidth: true; spacing: 6
                Label { text: 'Name' }
                Field {
                    id: nameField
                    Layout.fillWidth: true
                    Accessible.name: 'Name'
                    onTextEdited: form.title = text
                    onAccepted: form.save()
                }
            }
            ColumnLayout {
                Layout.fillWidth: true; spacing: 6
                Label { text: 'Kind' }
                Choice {
                    id: kindChoice
                    Layout.preferredWidth: 200
                    model: form.kinds
                    onActivated: index => form.kind = index
                    Accessible.name: 'Kind'
                }
            }
            ColumnLayout {
                Layout.fillWidth: true; spacing: 6
                visible: form.kind === 1
                Label { text: 'Usually' }
                RowLayout {
                    spacing: 10
                    Field {
                        id: usuallyField
                        Layout.preferredWidth: 80
                        inputMethodHints: Qt.ImhDigitsOnly
                        validator: IntValidator { bottom: 0; top: 999 }
                        Accessible.name: 'Usually'
                        onTextEdited: form.usually = text === '' ? '0' : text
                    }
                    LabelText { text: 'times a day'; color: Theme.muted }
                }
            }
            ColumnLayout {
                Layout.fillWidth: true; spacing: 6
                Label { text: 'Part of' }
                Choice {
                    id: placeChoice
                    Layout.preferredWidth: 200
                    model: form.places
                    onActivated: index => form.place = index
                    Accessible.name: 'Part of'
                }
            }
        }
        Item { Layout.fillHeight: true }
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: 16
            visible: form.dirty
            Item { Layout.fillWidth: true }
            SButton { text: 'Cancel'; quiet: true; onClicked: form.close() }
            SButton { text: 'Save'; quiet: true; busy: form.saving; enabled: !!form.title.trim(); onClicked: form.save() }
        }
    }
}
