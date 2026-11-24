import QtQuick
import QtQuick.Controls

Rectangle {
    id: chip
    objectName: "calendar-chip"

    property string day: record.date
    required property var record

    readonly property bool proposed: !!record.proposal_id
    // A task a habit relies on keeps its own event; the habit names itself
    // here and is never placed on the calendar as a task of its own.
    readonly property var habitTitles: (record.habit_ids || []).map(id => (Backend.habit(id) || {}).title).filter(t => !!t)
    color: proposed ? Theme.panel : Qt.tint(Theme.panel, Qt.alpha(record.color, .22))
    border.color: proposed ? Theme.accent : 'transparent'
    activeFocusOnTab: true
    Accessible.role: Accessible.Button
    Accessible.name: (proposed ? 'Proposed: ' : '') + record.title + (habitTitles.length ? ' · for ' + habitTitles.join(', ') : '')
    Keys.onReturnPressed: if (proposed) Backend.reviewProposalRequested(record.proposal_id); else Backend.select(record.id, record)
    height: 19
    radius: Theme.cornerRadius

    Rectangle {
        objectName: 'calendar-chip-habit'
        visible: (chip.record.habit_ids || []).length > 0
        width: 3; height: parent.height - 6
        x: 1; anchors.verticalCenter: parent.verticalCenter
        radius: 1
        color: Theme.accent
    }
    LabelText {
        anchors.fill: parent
        anchors.margins: 3
        anchors.leftMargin: (chip.record.habit_ids || []).length ? 6 : 3
        font.pixelSize: 10
        text: (chip.proposed ? '◇ Proposed · ' : '') + (chip.record.event_kind === 'due' ? '◇ ' : chip.record.event_kind === 'timed' ? (chip.record.local_start.substring(0, 10) < chip.day ? '↳ ' : chip.record.local_start.substring(11) + ' ') : '') + chip.record.title
    }
    CardDrag {
        card: chip
        draggable: !chip.proposed
        keys: ['calendar-event']
        record: chip.record

        onActivated: if(chip.proposed) Backend.reviewProposalRequested(chip.record.proposal_id); else Backend.select(chip.record.id, chip.record)
        onOpened: {
            if(chip.proposed) { Backend.reviewProposalRequested(chip.record.proposal_id); return; }
            Backend.select(chip.record.id, chip.record);
            Backend.detailFull = true;
        }
    }
}
