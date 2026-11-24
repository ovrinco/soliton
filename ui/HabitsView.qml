import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "CalendarMath.js" as Cal

// Hearth, Habits: the two bookends, the habits of the rest of the day and the
// map of past weeks, all read from the engine's habit-* calls.
FocusScope {
    id: view
    objectName: 'habits-view'

    readonly property var list: Backend.habitList
    function byIds(ids) { return ids.map(id => Backend.habit(id)).filter(h => !!h); }
    readonly property var morning: byIds(list.bundles.morning || [])
    readonly property var evening: byIds(list.bundles.evening || [])
    readonly property var through: list.habits.filter(h => !h.bundle)
    readonly property var flat: morning.concat(evening).concat(through)
    readonly property bool empty: Backend.habitsLoaded && list.habits.length === 0
    readonly property bool isToday: Backend.habitShownDay === Backend.today
    property string focusId: ''
    property var drop: null

    function headingDay() {
        var p = Backend.habitShownDay.split('-');
        var text = new Date(Number(p[0]), Number(p[1]) - 1, Number(p[2])).toLocaleDateString(Qt.locale('en_GB'), 'dddd d MMMM');
        return isToday ? 'Today, ' + text : text;
    }
    function moveDay(n) {
        var next = Cal.add(Backend.habitShownDay, n);
        if (next > Backend.today) return;
        Backend.habitDay = next === Backend.today ? '' : next;
    }
    function bundleOf(id) {
        var h = Backend.habit(id);
        return h ? h.bundle || '' : '';
    }
    function step(id, n) {
        var h = Backend.habit(id);
        if (!h) return;
        var v = Backend.habitValue(id);
        if (h.direction === 'break') Backend.habitCheckIn(id, Math.max(0, Math.round(v || 0) + n));
        else if (n > 0) Backend.habitCheckIn(id, v === 1 ? 0 : 1);
    }
    function nudge(id, n) {
        var b = bundleOf(id);
        if (!b) return;
        var order = (list.bundles[b] || []).slice(), i = order.indexOf(id), j = i + n;
        if (i < 0 || j < 0 || j >= order.length) return;
        order.splice(i, 1); order.splice(j, 0, id);
        Backend.habitOrder(b, order);
    }
    // Where a drag over the bookends would land: a bundle and an index in it.
    function dropAt(sceneX, sceneY, id) {
        var lists = [{bundle: 'morning', item: morningBookend.rows}, {bundle: 'evening', item: eveningBookend.rows}];
        for (var k = 0; k < lists.length; ++k) {
            var it = lists[k].item, p = it.mapFromItem(null, sceneX, sceneY);
            if (p.x < 0 || p.x > it.width || p.y < -Theme.denseRowHeight || p.y > it.height + Theme.denseRowHeight) continue;
            var ids = (list.bundles[lists[k].bundle] || []).filter(x => x !== id);
            var index = Math.max(0, Math.min(ids.length, Math.round(p.y / Theme.denseRowHeight)));
            return {bundle: lists[k].bundle, index: index};
        }
        return null;
    }
    function finishDrag(id, target) {
        drop = null;
        if (!target) return;
        var from = bundleOf(id);
        if (target.bundle !== from) { Backend.habitMove(id, target.bundle); return; }
        var order = (list.bundles[from] || []).filter(x => x !== id);
        order.splice(target.index, 0, id);
        if (JSON.stringify(order) !== JSON.stringify(list.bundles[from])) Backend.habitOrder(from, order);
    }

    Connections {
        target: Backend
        function onHabitFocusRequestChanged() { focusTimer.restart(); }
    }
    // "Check in: morning" and "Check in: evening" land on the first unchecked
    // row of that bundle once the day's values have arrived.
    Timer {
        id: focusTimer
        interval: 250
        onTriggered: {
            var b = Backend.habitFocusRequest.bundle, rows = b === 'evening' ? view.evening : view.morning;
            var first = rows.find(h => Backend.habitValue(h.id) === null || (h.direction !== 'break' && Backend.habitValue(h.id) !== 1));
            if (first || rows.length) view.focusId = (first || rows[0]).id;
            view.forceActiveFocus();
        }
    }
    Component.onCompleted: {
        if (Backend.habitFocusRequest.serial) focusTimer.restart();
        if (planHabitId) Backend.loadHabitPlan(planHabitId);
    }

    Keys.onPressed: event => {
        var alt = event.modifiers & Qt.AltModifier, shift = event.modifiers & Qt.ShiftModifier, ctrl = event.modifiers & Qt.ControlModifier;
        var i = view.flat.findIndex(h => h.id === view.focusId);
        if (alt && event.key === Qt.Key_Left) { moveDay(-1); event.accepted = true; }
        else if (alt && event.key === Qt.Key_Right) { moveDay(1); event.accepted = true; }
        else if (alt && (event.key === Qt.Key_Up || event.key === Qt.Key_Down)) { if (i >= 0) nudge(view.focusId, event.key === Qt.Key_Up ? -1 : 1); event.accepted = true; }
        else if (event.key === Qt.Key_Down || event.key === Qt.Key_Up) {
            if (view.flat.length) {
                var n = i < 0 ? 0 : Math.max(0, Math.min(view.flat.length - 1, i + (event.key === Qt.Key_Down ? 1 : -1)));
                view.focusId = view.flat[n].id;
            }
            event.accepted = true;
        }
        else if (event.key === Qt.Key_Space && i >= 0) { step(view.focusId, shift ? -1 : 1); event.accepted = true; }
        else if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter) && i >= 0) { Backend.openHabitForm(view.focusId); event.accepted = true; }
        else if (ctrl && event.key === Qt.Key_Z) { Backend.habitUndoLast(); event.accepted = true; }
    }

    component Bookend: ColumnLayout {
        id: bookend
        property string bundle: ''
        property string heading: ''
        property string emptyText: ''
        property var habits: []
        readonly property Item rows: rowColumn
        spacing: 6
        LabelText {
            text: bookend.heading
            font.pixelSize: Theme.sectionTitleSize; font.weight: Font.Medium
        }
        LabelText {
            visible: bookend.habits.length === 0
            text: bookend.emptyText
            color: Theme.muted
        }
        Column {
            id: rowColumn
            Layout.fillWidth: true
            Repeater {
                model: bookend.habits
                delegate: Item {
                    id: slot
                    required property var modelData
                    required property int index
                    width: rowColumn.width
                    height: Theme.denseRowHeight
                    Rectangle {
                        width: parent.width; height: 2; y: -1
                        color: Theme.accent
                        visible: !!view.drop && view.drop.bundle === bookend.bundle && view.drop.index === slot.index
                    }
                    Rectangle {
                        width: parent.width; height: 2; y: parent.height - 1
                        color: Theme.accent
                        visible: !!view.drop && view.drop.bundle === bookend.bundle && slot.index === bookend.habits.length - 1 && view.drop.index >= bookend.habits.length
                    }
                    HabitRow {
                        anchors.fill: parent
                        habit: slot.modelData
                        value: Backend.habitValue(slot.modelData.id)
                        focusedRow: view.activeFocus && view.focusId === slot.modelData.id
                        draggable: !!bookend.bundle
                        onActivated: { view.focusId = slot.modelData.id; view.forceActiveFocus(); }
                        onDragMoved: (y, x) => view.drop = bookend.bundle ? view.dropAt(x, y, slot.modelData.id) : null
                        onDragDropped: (y, x) => { if (bookend.bundle) view.finishDrag(slot.modelData.id, view.dropAt(x, y, slot.modelData.id)); else view.drop = null; }
                    }
                }
            }
        }
    }

    // The chosen habit's place in the plan: the same category, goal and task
    // pickers as the planner, and the day's actual task occurrences behind it.
    // Choosing here never checks the habit in.
    property string planId: ''
    readonly property var planHabit: Backend.habit(planId || focusId) || flat[0] || null
    readonly property string planHabitId: planHabit ? planHabit.id : ''
    readonly property var planLinks: Backend.habitPlan.id === planHabitId && Backend.habitPlan.hierarchy
        ? Backend.habitPlan.hierarchy : (planHabit && planHabit.hierarchy) || {category_id: null, goal_id: null, task_ids: []}
    onPlanHabitIdChanged: Backend.loadHabitPlan(planHabitId)
    Connections {
        target: Backend
        function onHabitListChanged() { if (view.planHabitId) Backend.loadHabitPlan(view.planHabitId); }
    }
    function goalCategory(goalId) {
        var g = goalId ? Backend.get(goalId) : null;
        return g ? g.category_id || '' : '';
    }
    function taskCategory(t) {
        return t.goal_id && Backend.get(t.goal_id) ? goalCategory(t.goal_id) : t.category_id || '';
    }
    // Tasks the engine accepts under the chosen parents: the goal's tasks, or
    // the category's tasks without a goal.
    function candidateTasks() {
        var category = planLinks.category_id || '', goal = planLinks.goal_id || '', linked = planLinks.task_ids || [];
        if (!category && !goal) return [];
        return Backend.records.filter(r => r.kind === 'task' && !r.hidden && linked.indexOf(r.id) < 0
            && (r.goal_id || '') === goal && taskCategory(r) === category).sort((a, c) => a.order - c.order);
    }
    // Tasks already linked that still sit under the chosen parents.
    function keptTasks(category, goal) {
        return (planLinks.task_ids || []).filter(id => {
            var t = Backend.get(id);
            return t && (t.goal_id || '') === (goal || '') && taskCategory(t) === (category || '');
        });
    }
    function relink(category, goal, taskIds) {
        Backend.habitLink(planHabitId, {category_id: category, goal_id: goal, task_ids: taskIds});
    }
    function actionDay(a) {
        var date = a.occurrence_date || a.date || (a.local_start || '').substring(0, 10) || a.scheduled_for || '';
        if (!a.on_timeline || !date) return 'Not on the calendar';
        var p = date.split('-'), d = new Date(Number(p[0]), Number(p[1]) - 1, Number(p[2]));
        var time = a.event_kind === 'timed' && a.local_start ? ' ' + a.local_start.substring(11, 16) : '';
        return d.toLocaleDateString(Qt.locale('en_GB'), 'ddd d MMM') + time;
    }

    component PlanLinks: ColumnLayout {
        id: plan
        objectName: 'habit-plan'
        spacing: 8
        readonly property string status: Backend.habitPlan.id === view.planHabitId ? Backend.habitPlan.hierarchy_status : (view.planHabit && view.planHabit.hierarchy_status) || 'unlinked'
        readonly property var linkedIds: view.planLinks.task_ids || []
        readonly property var actions: Backend.habitPlan.id === view.planHabitId ? Backend.habitPlan.actions || [] : []
        LabelText {
            text: 'In the plan'
            font.pixelSize: Theme.sectionTitleSize; font.weight: Font.Medium
        }
        GridLayout {
            Layout.fillWidth: true
            columns: 2; columnSpacing: 12; rowSpacing: 8
            LabelText { text: 'Habit'; color: Theme.muted; font.pixelSize: 13 }
            Choice {
                objectName: 'habit-plan-habit'
                Layout.fillWidth: true
                model: view.flat; textRole: 'title'
                currentIndex: Math.max(0, view.flat.findIndex(h => h.id === view.planHabitId))
                onActivated: view.planId = model[index].id
                Accessible.name: 'Habit'
            }
            LabelText { text: 'Category'; color: Theme.muted; font.pixelSize: 13 }
            Choice {
                objectName: 'habit-plan-category'
                Layout.fillWidth: true
                enabled: !view.planLinks.goal_id
                model: [{id:'', title:'Unassigned'}].concat(Backend.categories); textRole: 'title'
                currentIndex: Math.max(0, model.findIndex(c => c.id === (view.planLinks.category_id || '')))
                onActivated: view.relink(model[index].id, '', view.keptTasks(model[index].id, ''))
                Accessible.name: 'Category'
            }
            LabelText { text: 'Goal'; color: Theme.muted; font.pixelSize: 13 }
            Choice {
                objectName: 'habit-plan-goal'
                Layout.fillWidth: true
                model: [{id:'', title:'No goal'}].concat(Backend.goals.filter(g => !view.planLinks.category_id || view.planLinks.goal_id === g.id || g.category_id === view.planLinks.category_id)); textRole: 'title'
                currentIndex: Math.max(0, model.findIndex(g => g.id === (view.planLinks.goal_id || '')))
                onActivated: { var c = index ? model[index].category_id || '' : view.planLinks.category_id || ''; view.relink(c, model[index].id, view.keptTasks(c, model[index].id)); }
                Accessible.name: 'Goal'
            }
        }
        LabelText {
            Layout.fillWidth: true
            visible: plan.status === 'moved-parent' || plan.status === 'removed-parent'
            wrapMode: Text.WordWrap
            color: Theme.dangerText
            text: plan.status === 'moved-parent' ? 'A task or goal here has moved to another place in the plan. Choose its category and goal again.'
                : 'A category, goal or task here is no longer in the plan. Choose again, or restore it.'
        }
        LabelText {
            visible: plan.linkedIds.length === 0
            color: Theme.muted
            text: view.planLinks.category_id || view.planLinks.goal_id ? 'No tasks yet' : 'Not in the plan yet'
        }
        Repeater {
            model: plan.linkedIds
            delegate: RowLayout {
                id: linked
                required property string modelData
                required property int index
                readonly property var task: Backend.get(modelData)
                readonly property var steps: plan.actions.filter(a => a.id === linked.modelData)
                objectName: 'habit-plan-task-' + modelData
                Layout.fillWidth: true
                spacing: 6
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    LabelText { Layout.fillWidth: true; elide: Text.ElideRight; text: linked.task ? linked.task.title : 'Unavailable task' }
                    Repeater {
                        model: linked.steps
                        delegate: SCheckBox {
                            required property var modelData
                            required property int index
                            objectName: 'habit-plan-action-' + linked.modelData + '-' + index
                            text: view.actionDay(modelData)
                            checked: modelData.status === 'completed'
                            enabled: plan.status === 'linked'
                            onClicked: Backend.habitActionSet(view.planHabitId, modelData, checked)
                        }
                    }
                    LabelText {
                        visible: linked.steps.length === 0
                        color: Theme.muted; font.pixelSize: 12
                        text: 'Nothing on ' + (view.isToday ? 'this day' : 'the shown day')
                    }
                }
                SButton { objectName: 'habit-plan-up-' + linked.modelData; text: 'Earlier'; quiet: true; visible: linked.index > 0
                    onClicked: { var ids = plan.linkedIds.slice(); ids.splice(linked.index, 1); ids.splice(linked.index - 1, 0, linked.modelData); view.relink(view.planLinks.category_id, view.planLinks.goal_id, ids); } }
                SButton { objectName: 'habit-plan-open-' + linked.modelData; text: 'Open'; quiet: true; enabled: !!linked.task; onClicked: Backend.openHabitTask(linked.modelData) }
                SButton { objectName: 'habit-plan-remove-' + linked.modelData; text: 'Remove'; quiet: true
                    onClicked: view.relink(view.planLinks.category_id, view.planLinks.goal_id, plan.linkedIds.filter(x => x !== linked.modelData)) }
            }
        }
        Choice {
            objectName: 'habit-plan-add-task'
            visible: !!(view.planLinks.category_id || view.planLinks.goal_id) && plan.status !== 'removed-parent'
            Layout.fillWidth: true
            model: [{id:'', title:'Add a task'}].concat(view.candidateTasks()); textRole: 'title'
            currentIndex: 0
            onActivated: { if (index) view.relink(view.planLinks.category_id, view.planLinks.goal_id, plan.linkedIds.concat([model[index].id])); currentIndex = 0; }
            Accessible.name: 'Add a task'
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            spacing: 4
            Item { Layout.fillWidth: true }
            LabelText {
                visible: !view.empty
                text: view.headingDay()
                font.pixelSize: 14; font.weight: Font.Medium
                Layout.rightMargin: 8
            }
            SButton { visible: !view.empty; iconName: 'left'; accessibleLabel: 'Previous day'; quiet: true; onClicked: view.moveDay(-1) }
            SButton { visible: !view.empty && !view.isToday; iconName: 'right'; accessibleLabel: 'Next day'; quiet: true; onClicked: view.moveDay(1) }
            SButton { visible: !view.empty && !view.isToday; text: 'Today'; quiet: true; onClicked: Backend.habitDay = '' }
        }

        // Nothing to show: one muted line and the one button, quiet while
        // private mode is on.
        ColumnLayout {
            visible: view.empty
            Layout.alignment: Qt.AlignLeft
            spacing: 16
            LabelText { text: 'No habits yet'; color: Theme.muted }
            SButton { Layout.alignment: Qt.AlignLeft; text: 'Add habit'; primary: !Backend.privateMode; quiet: Backend.privateMode; onClicked: Backend.openHabitForm('') }
        }
        Item { Layout.fillHeight: true; visible: !scroll.visible }

        Flickable {
            id: scroll
            visible: !view.empty && Backend.habitsLoaded
            Layout.fillWidth: true; Layout.fillHeight: true
            clip: true
            contentWidth: width
            contentHeight: body.implicitHeight + 16
            boundsBehavior: Flickable.StopAtBounds
            interactive: !view.drop
            ScrollBar.vertical: SScrollBar { }

            ColumnLayout {
                id: body
                width: scroll.width - 12
                spacing: 28

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 32
                    Bookend {
                        id: morningBookend
                        Layout.fillWidth: true; Layout.preferredWidth: 1
                        Layout.alignment: Qt.AlignTop
                        bundle: 'morning'; heading: 'Morning'; emptyText: 'Nothing in the morning yet'
                        habits: view.morning
                    }
                    Bookend {
                        id: eveningBookend
                        Layout.fillWidth: true; Layout.preferredWidth: 1
                        Layout.alignment: Qt.AlignTop
                        bundle: 'evening'; heading: 'Evening'; emptyText: 'Nothing in the evening yet'
                        habits: view.evening
                    }
                }
                Bookend {
                    Layout.fillWidth: true
                    visible: view.through.length > 0
                    bundle: ''; heading: 'Through the day'
                    habits: view.through
                }
                HabitMap {
                    Layout.fillWidth: true
                    formIds: view.flat.filter(h => h.direction !== 'break').map(h => h.id)
                    breakIds: view.flat.filter(h => h.direction === 'break').map(h => h.id)
                }
                PlanLinks {
                    Layout.fillWidth: true
                    visible: view.flat.length > 0
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            visible: !view.empty
            Item { Layout.fillWidth: true }
            SButton { text: 'Add habit'; quiet: true; onClicked: Backend.openHabitForm('') }
        }
    }
}
