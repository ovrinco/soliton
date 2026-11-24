import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "CalendarMath.js" as Cal

Item {
    id: root
    property real completionClock: Date.now()
    Timer { interval: 250; repeat: true; running: root.visible && !!(Backend.prefs.lastTaskCompletion || {}).undoUntil && (Backend.prefs.lastTaskCompletion || {}).undoUntil > root.completionClock; onTriggered: root.completionClock = Date.now() }
    RetainedModel { id: calendarEvents; active: root.visible; sourceModel: Backend.filteredEvents.filter(e => Cal.showTask(Backend, e, root.completionClock)) }
    RetainedModel { id: calendarProposals; active: root.visible; sourceModel: Backend.proposalEvents }

    property string agendaDay: ''
    property bool backlog: false
    // Dashboard owns the horizon tabs; standalone calendar keeps its own.
    property bool embedded: false
    property var days: Backend.calendarMode === 'Day' ? [Backend.selectedDate] : Array.from({
        length: 7
    }, (_, i) => Cal.add(weekStart, i))
    property int endMinute: (Number(Backend.prefs.endHour) <= Number(Backend.prefs.startHour) ? Number(Backend.prefs.endHour) + 24 : Number(Backend.prefs.endHour)) * 60
    property real hourHeight: Number(Backend.prefs.hourHeight) || 58
    property var selected: Cal.parse(Backend.selectedDate)
    property int startMinute: Number(Backend.prefs.startHour) * 60
    property string weekStart: Cal.week(Backend.selectedDate, 0)

    function navigate(delta) {
        var d = Cal.parse(Backend.selectedDate);
        if (Backend.calendarMode === 'Year')
            d.setFullYear(d.getFullYear() + delta);
        else if (Backend.calendarMode === 'Month') {
            d.setDate(1);
            d.setMonth(d.getMonth() + delta);
        } else
            d.setDate(d.getDate() + delta * (Backend.calendarMode === 'Week' ? 7 : 1));
        Backend.selectedDate = Cal.iso(d);
    }
    function onDay(day) {
        return calendarEvents.model.filter(e => e.event_kind === 'timed' ? e.local_start.substring(0, 10) <= day && e.last_date >= day : e.date === day).concat(calendarProposals.model.filter(e => e.date === day));
    }
    function showDay(day) {
        agendaDay = day;
        agenda.open();
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        RowLayout {
            Layout.fillWidth: true

            LabelText {
                Layout.fillWidth: true
                font.pixelSize: 25
                font.weight: Font.DemiBold
                text: Backend.calendarMode === 'Year' ? String(root.selected.getFullYear()) : Qt.formatDate(root.selected, 'MMMM yyyy')
            }
            SButton {
                text: '‹'

                onClicked: root.navigate(-1)
            }
            SButton {
                text: 'Today'

                onClicked: Backend.selectedDate = Backend.today
            }
            SButton {
                text: '›'

                onClicked: root.navigate(1)
            }
            SButton {
                primary: root.backlog
                text: 'Unscheduled'

                onClicked: root.backlog = !root.backlog
            }
            SCheckBox { visible: !root.embedded; text: root.width > 820 ? 'Show completed' : ''; Accessible.name: 'Show completed'; checked: Backend.showCompleted; onToggled: { Backend.showCompleted = checked; Backend.remember(); } }
            SButton { text: 'Undo'; quiet: true; visible: Cal.canUndo(Backend, root.completionClock); onClicked: Cal.undoCompletion(Backend) }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 4
            visible: !root.embedded

            Repeater {
                model: ['Day', 'Week', 'Month', 'Year']

                delegate: SButton {
                    required property string modelData

                    primary: Backend.calendarMode === modelData
                    quiet: Backend.calendarMode !== modelData
                    text: modelData

                    onClicked: Backend.calendarMode = modelData
                }
            }
            Item {
                Layout.fillWidth: true
            }
            SButton {
                text: 'Proposals ('+calendarProposals.model.length+')'
                visible: calendarProposals.model.length>0
                onClicked: { Backend.strategyTab='Execution';Backend.navigate('Strategy'); }
            }
            LabelText {
                color: Theme.muted
                font.pixelSize: 11
                text: Backend.prefs.timezone
                visible: root.width > 730
            }
            SButton {
                quiet: true
                text: 'Hours & density'

                onClicked: settings.open()
            }
        }
        // Day and week share real time geometry and mutation commands.
        ColumnLayout {
            Layout.fillHeight: true
            Layout.fillWidth: true
            spacing: 0
            visible: Backend.calendarMode === 'Day' || Backend.calendarMode === 'Week'

            Row {
                Layout.fillWidth: true
                Layout.preferredHeight: 50
                leftPadding: 52

                Repeater {
                    model: root.days

                    Item {
                        id: dayHeader

                        required property string modelData

                        height: 50
                        width: (root.width - 52) / root.days.length

                        Row {
                            anchors.centerIn: parent
                            spacing: 7

                            LabelText {
                                anchors.verticalCenter: parent.verticalCenter
                                color: Theme.muted
                                font.pixelSize: 10
                                text: Qt.formatDate(Cal.parse(dayHeader.modelData), 'ddd').toUpperCase()
                            }
                            Rectangle {
                                color: dayHeader.modelData === Backend.today ? Theme.accent : 'transparent'
                                height: 28
                                radius: Theme.cornerRadius
                                width: 28

                                LabelText {
                                    anchors.centerIn: parent
                                    color: dayHeader.modelData === Backend.today ? Theme.accentForeground : Theme.text
                                    font.pixelSize: 16
                                    text: Cal.parse(dayHeader.modelData).getDate()
                                }
                            }
                        }
                        MouseArea {
                            anchors.fill: parent

                            onClicked: {
                                Backend.selectedDate = dayHeader.modelData;
                                Backend.calendarMode = 'Day';
                            }
                        }
                    }
                }
            }
            Row {
                Layout.fillWidth: true
                Layout.preferredHeight: 72

                LabelText {
                    color: Theme.muted
                    font.pixelSize: 9
                    height: 72
                    text: 'ALL DAY\n& DUE'
                    verticalAlignment: Text.AlignVCenter
                    width: 52
                }
                Repeater {
                    model: root.days

                    Rectangle {
                        id: allDay

                        property var items: calendarEvents.model.concat(calendarProposals.model).filter(e => e.date === modelData && e.event_kind !== 'timed')
                        required property string modelData

                        border.color: Theme.line
                        color: Theme.panel
                        height: 72
                        width: (root.width - 52) / root.days.length

                        ListView {
                            id: allDayList
                            boundsBehavior: Flickable.StopAtBounds
                            objectName: 'calendar-all-day-list'
                            property string day: allDay.modelData
                            anchors.fill: parent
                            anchors.margins: 3
                            clip: true
                            interactive: !Backend.draggingCard && contentHeight > height
                            spacing: 3
                            model: allDay.items
                            delegate: CalendarChip {
                                required property var modelData
                                record: modelData
                                day: allDay.modelData
                                width: allDayList.width
                                height: 20
                            }
                            ScrollBar.vertical: ScrollBar {}
                        }
                        DropArea {
                            anchors.fill: parent
                            keys: ['task', 'calendar-event']

                            onDropped: event => {
                                if (event.source && event.source.dropHandled !== undefined)
                                    event.source.dropHandled = true;
                                Backend.moveToAllDay(event.source.record, allDay.modelData);
                                event.acceptProposedAction();
                            }
                        }
                    }
                }
            }
            Flickable {
                id: timeline
                objectName: "calendar-timeline"

                Layout.fillHeight: true
                Layout.fillWidth: true
                boundsBehavior: Flickable.StopAtBounds
                clip: true
                contentHeight: (root.endMinute - root.startMinute) / 60 * root.hourHeight
                contentWidth: width

                ScrollBar.vertical: ScrollBar {}

                Repeater {
                    model: Math.ceil((root.endMinute - root.startMinute) / 60) + 1

                    Item {
                        required property int index

                        height: 1
                        width: timeline.width
                        y: index * root.hourHeight

                        Rectangle {
                            color: Theme.line
                            height: 1
                            opacity: .65
                            width: parent.width - 52
                            x: 52
                        }
                        LabelText {
                            color: Theme.muted
                            font.pixelSize: 10
                            horizontalAlignment: Text.AlignRight
                            text: Cal.clock(root.startMinute + parent.index * 60)
                            width: 44
                            y: 4
                        }
                    }
                }
                Row {
                    height: timeline.contentHeight
                    x: 52

                    Repeater {
                        model: root.days

                        Item {
                            id: dayColumn
                            objectName: "calendar-day-column"

                            required property string modelData
                            property var packed: Cal.layout(calendarEvents.model, modelData, root.startMinute, root.endMinute)

                            height: timeline.contentHeight
                            width: (timeline.width - 52) / root.days.length

                            Rectangle {
                                color: Theme.line
                                height: parent.height
                                opacity: .65
                                width: 1
                            }
                            Rectangle {
                                anchors.fill: parent
                                color: Theme.accent
                                opacity: dayColumn.modelData === Backend.today ? .025 : 0
                            }
                            Rectangle {
                                color: Theme.panel
                                height: 18
                                radius: Theme.cornerRadius
                                visible: root.endMinute > 1440
                                width: Math.min(110, parent.width - 10)
                                x: Math.max(5, parent.width - width - 5)
                                y: (1440 - root.startMinute) / 60 * root.hourHeight + 3
                                z: 45

                                LabelText {
                                    anchors.centerIn: parent
                                    color: Theme.accent
                                    font.pixelSize: 10
                                    text: Qt.formatDate(Cal.parse(Cal.add(dayColumn.modelData, 1)), 'ddd, MMM d')
                                }
                            }
                            MouseArea {
                                id: createArea

                                property real beginY: 0
                                property real endY: 0

                                anchors.fill: parent

                                onPositionChanged: mouse => {
                                    if (pressed)
                                        endY = mouse.y;
                                }
                                onPressed: mouse => {
                                    beginY = mouse.y;
                                    endY = mouse.y;
                                }
                                onReleased: mouse => {
                                    var minute = root.startMinute + Math.floor(Math.min(beginY, mouse.y) / root.hourHeight * 60 / 15) * 15;
                                    var duration = Math.max(15, Math.round(Math.abs(mouse.y - beginY) / root.hourHeight * 60 / 15) * 15);
                                    if (Math.abs(mouse.y - beginY) < 8)
                                        duration = 60;
                                    Backend.create('task', '', '', Cal.add(dayColumn.modelData, Math.floor(minute / 1440)), Cal.clock(minute));
                                    Backend.editSchedule('duration_minutes', duration);
                                }
                            }
                            Rectangle {
                                color: Theme.accent
                                height: Math.max(root.hourHeight / 4, Math.abs(createArea.endY - createArea.beginY))
                                opacity: .2
                                visible: createArea.pressed
                                width: parent.width - 4
                                x: 2
                                y: Math.min(createArea.beginY, createArea.endY)
                            }
                            Repeater {
                                model: dayColumn.packed

                                delegate: EventBlock {
                                    required property var modelData

                                    columnWidth: dayColumn.width
                                    day: dayColumn.modelData
                                    entry: modelData
                                    hourHeight: root.hourHeight
                                    startMinute: root.startMinute

                                    // The task's own event, marked when a habit relies on it.
                                    Rectangle {
                                        objectName: 'event-block-habit'
                                        visible: (modelData.event.habit_ids || []).length > 0
                                        width: 3; height: parent.height - 4
                                        x: 1; y: 2; z: 3
                                        radius: 1
                                        color: Theme.accent
                                    }
                                }
                            }
                            Repeater {
                                model: Cal.layout(calendarProposals.model,dayColumn.modelData,root.startMinute,root.endMinute)
                                delegate: Rectangle {
                                    required property var modelData
                                    objectName:'proposal-calendar-block'
                                    x:dayColumn.width*.55+modelData.column*(dayColumn.width*.45)/modelData.columns
                                    y:(modelData.start-root.startMinute)*root.hourHeight/60
                                    width:Math.max(20,dayColumn.width*.45/modelData.columns-4)
                                    height:Math.max(20,(modelData.end-modelData.start)*root.hourHeight/60)
                                    color:Theme.panel;border.color:Theme.accent;border.width:2;radius: Theme.cornerRadius;z:5
                                    LabelText { anchors.fill:parent;anchors.margins:4;text:'◇ Proposed · '+modelData.event.title;textFormat:Text.PlainText;font.pixelSize:11;color:Theme.accent;wrapMode:Text.Wrap;clip:true }
                                    Accessible.role:Accessible.Button
                                    Accessible.name:'Review proposed work: '+modelData.event.title
                                    activeFocusOnTab:true
                                    Keys.onReturnPressed:Backend.reviewProposalRequested(modelData.event.proposal_id)
                                    MouseArea { anchors.fill:parent;cursorShape:Qt.PointingHandCursor;onClicked:Backend.reviewProposalRequested(modelData.event.proposal_id) }
                                }
                            }
                            DropArea {
                                id: taskDrop

                                anchors.fill: parent
                                keys: ['task', 'calendar-event']

                                onDropped: event => {
                                    if (event.source && event.source.dropHandled !== undefined)
                                        event.source.dropHandled = true;
                                    var pointerX = event.x;
                                    var pointerY = event.y;
                                    if (event.source && event.source.grabPoint) {
                                        var pointer = event.source.mapToItem(taskDrop, event.source.grabPoint.x, event.source.grabPoint.y);
                                        pointerX = pointer.x;
                                        pointerY = pointer.y;
                                    }
                                    // The Flickable content can extend beyond
                                    // its clipped viewport. A release outside
                                    // the visible day column is a no-op even
                                    // if the content DropArea saw the drag.
                                    var visibleY = pointerY - timeline.contentY;
                                    if (pointerX < 0 || pointerX > taskDrop.width || visibleY < 0 || visibleY > timeline.height)
                                        return;
                                    // EventBlock's hotspot is part of the
                                    // source-local drag geometry.  Subtract it
                                    // for calendar events so grabbing 20px
                                    // below the top still lands on the slot
                                    // under the pointer. Card/task drags keep
                                    // direct pointer semantics.
                                    if (event.source && event.source.calendarDrag)
                                        pointerY -= event.source.grabPoint.y;
                                    var minute = Math.min(root.endMinute - 15, Math.max(root.startMinute, root.startMinute + Math.floor(pointerY / root.hourHeight * 60 / 15) * 15));
                                    Backend.moveEvent(event.source.record, Cal.add(dayColumn.modelData, Math.floor(minute / 1440)), minute);
                                    event.acceptProposedAction();
                                }

                                Rectangle {
                                    color: Theme.accent
                                    height: 2
                                    visible: taskDrop.containsDrag
                                    width: parent.width
                                    y: Math.floor(taskDrop.drag.y / root.hourHeight * 4) * root.hourHeight / 4
                                    z: 50

                                    LabelText {
                                        anchors.right: parent.right
                                        color: Theme.accent
                                        font.pixelSize: 11
                                        text: Cal.clock(root.startMinute + Math.floor(taskDrop.drag.y / root.hourHeight * 4) * 15)
                                        y: -18
                                    }
                                }
                            }
                            Rectangle {
                                property real minute: Cal.minuteInDay(Backend.nowLocal, dayColumn.modelData)

                                color: Theme.danger
                                height: 2
                                visible: minute >= root.startMinute && minute <= root.endMinute
                                width: parent.width
                                y: (minute - root.startMinute) / 60 * root.hourHeight
                                z: 30

                                Rectangle {
                                    color: Theme.danger
                                    height: 8
                                    radius: Theme.cornerRadius
                                    width: 8
                                    x: -3
                                    y: -3
                                }
                            }
                        }
                    }
                }
            }
        }
        // Month grid keeps overflow accessible through a day agenda.
        ColumnLayout {
            Layout.fillHeight: true
            Layout.fillWidth: true
            spacing: 0
            visible: Backend.calendarMode === 'Month'

            Row {
                Layout.fillWidth: true
                Layout.preferredHeight: 30

                Repeater {
                    model: 7

                    delegate: LabelText {
                        required property int index

                        color: Theme.muted
                        font.pixelSize: 10
                        horizontalAlignment: Text.AlignHCenter
                        text: Qt.formatDate(Cal.parse(Cal.add(root.weekStart, index)), 'ddd').toUpperCase()
                        width: root.width / 7
                    }
                }
            }
            GridLayout {
                Layout.fillHeight: true
                Layout.fillWidth: true
                columnSpacing: 1
                columns: 7
                rowSpacing: 1

                Repeater {
                    model: Cal.monthDays(Backend.selectedDate, 0)

                    Rectangle {
                        id: cell

                        property var items: root.onDay(modelData)
                        property int maxChips: Math.max(1, Math.floor((height - 44) / 22))
                        required property string modelData

                        Layout.fillHeight: true
                        Layout.fillWidth: true
                        Layout.minimumHeight: 60
                        Layout.minimumWidth: 0
                        color: modelData === Backend.today ? Theme.selection : Theme.panel
                        opacity: Cal.parse(modelData).getMonth() === root.selected.getMonth() ? 1 : .5

                        MouseArea {
                            anchors.fill: parent

                            onClicked: root.showDay(cell.modelData)
                            onDoubleClicked: {
                                Backend.selectedDate = cell.modelData;
                                Backend.calendarMode = 'Day';
                            }
                        }
                        Column {
                            anchors.fill: parent
                            anchors.margins: 6
                            spacing: 3

                            LabelText {
                                color: cell.modelData === Backend.today ? Theme.accent : Theme.text
                                font.pixelSize: 12
                                height: 22
                                text: Cal.parse(cell.modelData).getDate()
                            }
                            Repeater {
                                model: cell.items.slice(0, cell.maxChips)

                                delegate: CalendarChip {
                                    required property var modelData

                                    day: cell.modelData
                                    record: modelData
                                    width: parent.width
                                }
                            }
                            LabelText {
                                color: Theme.accent
                                font.pixelSize: 10
                                text: '+' + (cell.items.length - cell.maxChips) + ' more'
                                visible: cell.items.length > cell.maxChips

                                MouseArea {
                                    anchors.fill: parent

                                    onClicked: root.showDay(cell.modelData)
                                }
                            }
                        }
                        DropArea {
                            anchors.fill: parent
                            keys: ['task', 'calendar-event']

                            onDropped: event => {
                                if (event.source && event.source.dropHandled !== undefined)
                                    event.source.dropHandled = true;
                                Backend.moveToDate(event.source.record, cell.modelData);
                                event.acceptProposedAction();
                            }
                        }
                    }
                }
            }
        }
        Flickable {
            id: year

            Layout.fillHeight: true
            Layout.fillWidth: true
            boundsBehavior: Flickable.StopAtBounds
            clip: true
            contentHeight: yearGrid.height
            contentWidth: width
            visible: Backend.calendarMode === 'Year'

            ScrollBar.vertical: ScrollBar {}

            GridLayout {
                id: yearGrid

                columnSpacing: 12
                columns: year.width >= 1180 ? 4 : year.width >= 850 ? 3 : 2
                rowSpacing: 12
                width: year.width

                Repeater {
                    model: 12

                    Rectangle {
                        id: month

                        property string first: root.selected.getFullYear() + '-' + String(index + 1).padStart(2, '0') + '-01'
                        required property int index
                        property var monthEvents: calendarEvents.model.concat(calendarProposals.model).filter(e => e.date.substring(0, 7) === first.substring(0, 7))

                        Layout.fillWidth: true
                        Layout.preferredHeight: Math.max(190, Math.min(250, (year.height - 12 * (Math.ceil(12 / yearGrid.columns) - 1)) / Math.ceil(12 / yearGrid.columns)))
                        border.color: Theme.line
                        color: Theme.panel
                        radius: Theme.cornerRadius

                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 12
                            spacing: 8

                            RowLayout {
                                Layout.fillWidth: true

                                SButton {
                                    quiet: true
                                    text: Qt.formatDate(Cal.parse(month.first), 'MMMM')

                                    onClicked: {
                                        Backend.selectedDate = month.first;
                                        Backend.calendarMode = 'Month';
                                    }
                                }
                                Item {
                                    Layout.fillWidth: true
                                }
                                LabelText {
                                    color: Theme.muted
                                    font.pixelSize: 10
                                    text: month.monthEvents.length + ' items'
                                }
                            }
                            GridLayout {
                                Layout.fillHeight: true
                                Layout.fillWidth: true
                                columnSpacing: 2
                                columns: 7
                                rowSpacing: 2

                                Repeater {
                                    model: Cal.monthDays(month.first, 0)

                                    Rectangle {
                                        id: yearDay

                                        property var items: root.onDay(modelData)
                                        property int proposedCount: items.filter(e=>!!e.proposal_id).length
                                        property int count: items.length
                                        border.color: proposedCount ? Theme.accent : 'transparent'
                                        border.width: proposedCount ? 1 : 0
                                        required property string modelData

                                        Layout.fillHeight: true
                                        Layout.fillWidth: true
                                        color: count ? Qt.alpha(Theme.accent, Math.min(.45, .1 + count * .05)) : 'transparent'
                                        opacity: modelData.substring(0, 7) === month.first.substring(0, 7) ? 1 : .2
                                        radius: Theme.cornerRadius

                                        LabelText {
                                            anchors.centerIn: parent
                                            color: yearDay.modelData === Backend.today ? Theme.accent : Theme.text
                                            font.pixelSize: 10
                                            text: Cal.parse(yearDay.modelData).getDate()
                                        }
                                        MouseArea {
                                            Hint {
                                                text: yearDay.modelData + ' · ' + (yearDay.count-yearDay.proposedCount) + ' planned · ' + yearDay.proposedCount + ' proposed'
                                                visible: parent.containsMouse
                                            }
                                            anchors.fill: parent
                                            hoverEnabled: true

                                            onClicked: {
                                                Backend.selectedDate = yearDay.modelData;
                                                Backend.calendarMode = 'Day';
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 104
            spacing: 7
            visible: root.backlog

            LabelText {
                color: Theme.muted
                font.letterSpacing: 1
                font.pixelSize: 10
                text: 'UNSCHEDULED'
            }
            ListView {
                id: backlogList

                Layout.fillHeight: true
                Layout.fillWidth: true
                clip: true
                model: Backend.tasks.filter(t => !t.scheduled_for && t.recurrence === 'none' && Cal.showTask(Backend, t, root.completionClock))
                orientation: ListView.Horizontal
                spacing: 8

                ScrollBar.horizontal: ScrollBar {}
                delegate: TaskCard {
                    required property var modelData

                    compact: true
                    record: modelData
                    width: 260
                }
            }
        }
    }
    Popup {
        id: agenda

        // Private mode turning on closes every open popup, so none stays open on
        // an item that has just left the screen.
        Connections {
            target: Backend
            function onPrivateModeChanged() { if (Backend.privateMode && agenda.visible) agenda.close(); }
        }

        anchors.centerIn: parent
        focus: true
        height: Math.min(570, root.height - 24)
        modal: true
        padding: 18
        width: Math.min(420, root.width - 24)

        background: Rectangle {
            border.color: Theme.line
            color: Theme.panel
            radius: Theme.cornerRadius
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 12

            RowLayout {
                Layout.fillWidth: true

                LabelText {
                    Layout.fillWidth: true
                    font.pixelSize: 21
                    text: Qt.formatDate(Cal.parse(root.agendaDay || Backend.selectedDate), 'ddd, MMM d')
                }
                SButton {
                    quiet: true
                    text: '×'

                    onClicked: agenda.close()
                }
            }
            RowLayout {
                SButton {
                    primary: true
                    text: 'Open Day'

                    onClicked: {
                        Backend.selectedDate = root.agendaDay;
                        Backend.calendarMode = 'Day';
                        agenda.close();
                    }
                }
                SButton {
                    text: '+ Task'

                    onClicked: {
                        Backend.create('task', '', '', root.agendaDay);
                        agenda.close();
                    }
                }
            }
            ListView {
                id: agendaList

                Layout.fillHeight: true
                Layout.fillWidth: true
                clip: true
                model: root.onDay(root.agendaDay)
                spacing: 8

                ScrollBar.vertical: ScrollBar {}
                delegate: Rectangle {
                    id: agendaRow

                    required property var modelData

                    color: Theme.raised
                    height: 70
                    radius: Theme.cornerRadius
                    width: agendaList.width

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 10

                        LabelText {
                            Layout.fillWidth: true
                            text: (agendaRow.modelData.proposal_id ? '◇ Proposed · ' : '') + agendaRow.modelData.title
                        }
                        LabelText {
                            color: Theme.muted
                            font.pixelSize: 11
                            text: agendaRow.modelData.event_kind === 'timed' ? (agendaRow.modelData.local_start.substring(0, 10) < root.agendaDay ? 'Continues until ' + agendaRow.modelData.local_end.replace('T', ' ') : agendaRow.modelData.local_start.substring(11) + ' · ' + agendaRow.modelData.schedule.duration_minutes + ' min') : agendaRow.modelData.event_kind === 'due' ? 'Due date' : 'All day'
                        }
                    }
                    MouseArea {
                        anchors.fill: parent

                        onClicked: {
                            if(agendaRow.modelData.proposal_id) Backend.reviewProposalRequested(agendaRow.modelData.proposal_id);
                            else Backend.select(agendaRow.modelData.id, agendaRow.modelData);
                            agenda.close();
                        }
                    }
                }
            }
        }
    }
    Popup {
        id: settings

        // Private mode turning on closes every open popup, so none stays open on
        // an item that has just left the screen.
        Connections {
            target: Backend
            function onPrivateModeChanged() { if (Backend.privateMode && settings.visible) settings.close(); }
        }

        anchors.centerIn: parent
        focus: true
        height: 290
        modal: true
        padding: 20
        width: 330

        background: Rectangle {
            border.color: Theme.line
            color: Theme.panel
            radius: Theme.cornerRadius
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 12

            LabelText {
                font.pixelSize: 19
                text: 'Planning hours'
            }
            RowLayout {
                LabelText {
                    color: Theme.muted
                    text: 'From'
                }
                Choice {
                    Layout.fillWidth: true
                    currentIndex: Backend.prefs.startHour
                    model: Array.from({
                        length: 24
                    }, (_, i) => Cal.clock(i * 60))

                    onActivated: Backend.setting('startHour', currentIndex)
                }
                LabelText {
                    color: Theme.muted
                    text: 'to'
                }
                Choice {
                    Layout.fillWidth: true
                    currentIndex: Backend.prefs.endHour
                    model: Array.from({
                        length: 25
                    }, (_, i) => i === 24 ? '24:00' : Cal.clock(i * 60))

                    onActivated: Backend.setting('endHour', currentIndex)
                }
            }
            LabelText {
                Layout.fillWidth: true
                color: Theme.muted
                font.pixelSize: 11
                text: 'An end before the start continues into tomorrow.'
                wrapMode: Text.Wrap
            }
            RowLayout {
                LabelText {
                    color: Theme.muted
                    text: 'Density'
                }
                Slider {
                    Layout.fillWidth: true
                    from: 32
                    stepSize: 2
                    to: 100
                    value: Backend.prefs.hourHeight

                    onMoved: Backend.setting('hourHeight', Math.round(value))
                }
            }
            Field {
                Layout.fillWidth: true
                placeholderText: 'IANA timezone'
                text: Backend.prefs.timezone

                onEditingFinished: Backend.setting('timezone', text)
            }
            SButton {
                Layout.alignment: Qt.AlignRight
                primary: true
                text: 'Done'

                onClicked: settings.close()
            }
        }
    }
}
