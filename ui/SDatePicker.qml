pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

SPopup {
    id: root

    property string selectedDate: ''
    property date monthCursor: new Date()
    signal dateSelected(string isoDate)

    width: 304
    height: 376
    parent: Overlay.overlay
    padding: 12
    modal: false
    focus: true

    onAboutToShow: {
        var parts = selectedDate.split('-');
        var selected = parts.length === 3
            ? new Date(Number(parts[0]), Number(parts[1]) - 1, Number(parts[2])) : new Date();
        monthCursor = new Date(selected.getFullYear(), selected.getMonth(), 1);
    }

    function iso(date) {
        return date.getFullYear() + '-' + String(date.getMonth() + 1).padStart(2, '0') + '-' + String(date.getDate()).padStart(2, '0');
    }
    function cells() {
        var first = new Date(monthCursor.getFullYear(), monthCursor.getMonth(), 1);
        var start = new Date(first.getFullYear(), first.getMonth(), 1 - first.getDay());
        var result = [];
        for (var i = 0; i < 42; ++i) {
            var day = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i);
            result.push({date: day, iso: iso(day), inMonth: day.getMonth() === monthCursor.getMonth()});
        }
        return result;
    }
    function shiftMonth(amount) {
        monthCursor = new Date(monthCursor.getFullYear(), monthCursor.getMonth() + amount, 1);
    }
    function choose(value) {
        dateSelected(value);
        close();
    }

    background: Rectangle { color: Theme.panel; border.color: Theme.edge; radius: Theme.chromeRadius }
    contentItem: ColumnLayout {
        spacing: 8
        RowLayout {
            Layout.fillWidth: true
            SButton { iconName: 'left'; accessibleLabel: 'Previous month'; quiet: true; onClicked: root.shiftMonth(-1) }
            LabelText {
                Layout.fillWidth: true
                text: Qt.formatDate(root.monthCursor, 'MMMM yyyy')
                horizontalAlignment: Text.AlignHCenter
            }
            SButton { iconName: 'right'; accessibleLabel: 'Next month'; quiet: true; onClicked: root.shiftMonth(1) }
        }
        GridLayout {
            Layout.fillWidth: true
            columns: 7
            Repeater {
                model: ['S', 'M', 'T', 'W', 'T', 'F', 'S']
                delegate: LabelText {
                    required property string modelData
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    color: Theme.muted
                    text: modelData
                }
            }
            Repeater {
                model: root.cells()
                delegate: Button {
                    id: dayButton
                    required property var modelData
                    Layout.fillWidth: true
                    Layout.preferredHeight: 34
                    text: String(dayButton.modelData.date.getDate())
                    hoverEnabled: true
                    focusPolicy: Qt.StrongFocus
                    Accessible.name: Qt.formatDate(dayButton.modelData.date, 'dddd, MMMM d, yyyy')
                    background: Rectangle {
                        radius: Theme.chromeRadius
                        color: dayButton.modelData.iso === root.selectedDate ? Theme.selection
                            : dayButton.hovered ? Theme.hover : 'transparent'
                        border.width: dayButton.modelData.iso === root.iso(new Date()) ? 1 : 0
                        border.color: Theme.accent
                    }
                    contentItem: Text {
                        text: dayButton.text
                        color: !dayButton.modelData.inMonth ? Theme.muted : Theme.text
                        font: dayButton.font
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }
                    onClicked: root.choose(dayButton.modelData.iso)
                }
            }
        }
        Item { Layout.fillHeight: true }
        RowLayout {
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            SButton { text: 'Today'; quiet: true; onClicked: root.choose(root.iso(new Date())) }
        }
    }
}
