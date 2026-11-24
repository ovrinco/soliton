pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "CalendarMath.js" as Cal

Item {
    id: root
    objectName: 'tasks-view'
    property real completionClock: Date.now()
    Timer { interval: 250; repeat: true; running: root.visible && !!(Backend.prefs.lastTaskCompletion || {}).undoUntil && (Backend.prefs.lastTaskCompletion || {}).undoUntil > root.completionClock; onTriggered: root.completionClock = Date.now() }
    // A completed task stays in reach: its card holds it until Tasks is left
    // (TaskCard), the open task always stays, and Show completed brings back the rest.
    RetainedModel { id: tasksModel; active: root.visible; sourceModel: Backend.tasks.filter(t => Cal.showTask(Backend, t, root.completionClock) || t.id === Backend.selectedId) }

    // Saved views live in the workspace preferences (Backend.prefs.savedViews),
    // the list the Dashboard's saved-view drop-down reads.
    readonly property var savedViews: Backend.prefs.savedViews || []
    // A view is the one shown only when its filters and its layout both match what is on screen.
    // A field a view never recorded leaves that part of the screen as it is.
    readonly property string activeSavedViewName: {
        var current = [Backend.branches, Backend.priority, Backend.status, Backend.search];
        var found = savedViews.find(v => JSON.stringify([v.branches || [], v.priority || 'all', v.status || 'all', v.search || '']) === JSON.stringify(current)
            && (v.board === undefined || !!v.board === Backend.board)
            && (v.showArchived === undefined || !!v.showArchived === Backend.showArchived)
            && (v.showCompleted === undefined || !!v.showCompleted === Backend.showCompleted));
        return found ? found.name : '';
    }
    function saveCurrentView(name) {
        name = name.trim();
        if (!name) return;
        var list = savedViews.filter(v => v.name !== name);
        list.push({name: name, page: Backend.page, view: Backend.view, branches: Backend.branches.slice(),
            priority: Backend.priority, status: Backend.status, search: Backend.search, board: Backend.board,
            calendarMode: Backend.calendarMode, showArchived: Backend.showArchived, showCompleted: Backend.showCompleted});
        Backend.setting('savedViews', list);
        Backend.message('Saved view · ' + name);
    }
    function applySavedView(v) {
        Backend.applyView(v);
        if (v.showCompleted !== undefined) Backend.showCompleted = !!v.showCompleted;
    }

    property var selectedTaskIds: []
    property string lastSelectedId: ''
    property bool selectionMode: false
    property bool selectionRouteLeft: false
    readonly property string scopeKey: JSON.stringify([Backend.branches, Backend.priority, Backend.status, Backend.search, Backend.showArchived, Backend.showCompleted])
    property string selectionScope: ''
    function beginSelection() {
        selectionRouteLeft = false;
        selectionScope = scopeKey;
        selectionMode = true;
        forceActiveFocus();
    }
    function endSelection() {
        if (batchBusy) return;
        deselectAll(); selectionMode = false;
    }
    onScopeKeyChanged: if (selectionMode && visible && !Backend.restoringRoute && selectionScope !== scopeKey) endSelection()
    Connections {
        target: Backend
        function onRouteLeaving() {
            if (!root.selectionMode) return;
            root.selectionRouteLeft = true;
            if (!root.batchBusy) root.endSelection();
        }
    }
    onVisibleChanged: {
        completionClock = Date.now();
        if (!visible && selectionMode && !batchBusy) endSelection();
    }
    onBatchBusyChanged: if (!batchBusy && selectionRouteLeft) Qt.callLater(function() {
        if (root.selectionRouteLeft && !root.batchBusy) root.endSelection();
    })
    Keys.onEscapePressed: event => {
        if (selectionMode && !batchBusy) { endSelection(); event.accepted = true; }
        else event.accepted = false;
    }
    Keys.onPressed: event => {
        if (selectionMode && !batchBusy && event.key === Qt.Key_A && (event.modifiers & Qt.ControlModifier)) {
            selectAll(); event.accepted = true;
        }
    }

    function toggleSelect(task, index, shiftKey, orderedTasks) {
        if (!selectionMode || batchBusy || !task || !task.id)
            return;
        var list = root.selectedTaskIds.slice();
        var anchor = orderedTasks.findIndex(t => t.id === root.lastSelectedId);
        if (shiftKey && anchor >= 0) {
            var start = Math.min(anchor, index);
            var end = Math.max(anchor, index);
            for (var i = start; i <= end; i++) {
                var tid = orderedTasks[i].id;
                if (list.indexOf(tid) < 0)
                    list.push(tid);
            }
        } else {
            var pos = list.indexOf(task.id);
            if (pos >= 0)
                list.splice(pos, 1);
            else
                list.push(task.id);
        }
        root.lastSelectedId = task.id;
        root.selectedTaskIds = list;
    }

    function selectAll() {
        root.selectedTaskIds = tasksModel.model.map(t => t.id);
    }

    function deselectAll() {
        root.selectedTaskIds = [];
        root.lastSelectedId = '';
    }

    property bool batchBusy: false
    function applyBatch(operation, onFinished) {
        if (batchBusy || !selectedTaskIds.length) return;
        var ids = selectedTaskIds.slice(), failed = [], updated = 0, index = 0;
        batchBusy = true;
        function next() {
            if (index >= ids.length) {
                root.batchBusy = false;
                root.selectedTaskIds = failed;
                root.lastSelectedId = '';
                if (onFinished) onFinished(updated, failed);
                Backend.message('Updated ' + updated + ' of ' + ids.length + ' tasks' +
                    (failed.length ? ' · ' + failed.length + (root.selectionRouteLeft ? ' failed after leaving Tasks' : ' failed; selection retained') : ''), failed.length > 0);
                return;
            }
            var id = ids[index++], record = Backend.get(id);
            if (!record) { failed.push(id); next(); return; }
            operation(record, function(value, ok) {
                if (ok) updated++; else failed.push(id);
                next();
            });
        }
        next();
    }
    function applyBatchStatus(status) {
        var receipts = [];
        applyBatch((record, done) => Cal.completion(Backend, record, status, done, entry => receipts.push(entry)),
            function() {
                if (receipts.length) {
                    var now = Date.now();
                    Backend.setting('lastTaskCompletion', {entries:receipts, graceUntil:now + 4000, undoUntil:now + 30000});
                }
            });
    }
    function applyBatchPriority(priority) {
        applyBatch((record, done) => Backend.mutate(record, {priority:priority}, done));
    }
    function applyBatchGoal(goalId) {
        var goal = Backend.get(goalId);
        applyBatch((record, done) => Backend.mutate(record,
            {goal_id:goalId, category_id:goal ? goal.category_id : record.category_id}, done));
    }
    // Owner is an owner:human or owner:agent tag; any earlier owner tag is replaced.
    function applyBatchOwner(owner) {
        applyBatch((record, done) => Backend.mutate(record,
            {tags: (record.tags || []).filter(t => t.indexOf('owner:') !== 0).concat(['owner:' + owner])}, done));
    }
    function applyBatchArchive() {
        applyBatch((record, done) => Backend.mutate(record, {archived:true}, done));
    }
    SDialog {
        id: archiveReview
        title: 'Archive selected tasks'
        anchors.centerIn: Overlay.overlay
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        LabelText { text: 'Archive ' + root.selectedTaskIds.length + ' selected tasks? History remains available.' }
        onAccepted: root.applyBatchArchive()
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 10

        // One 40px action band in both normal and Select mode.
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.actionHeight
            RowLayout {
                anchors.fill: parent; spacing: 8; visible: !root.selectionMode
                SSearchField { Layout.fillWidth: true; Layout.minimumWidth: 40; frameless: true; scopeName: 'tasks'; text: Backend.search; onTextEdited: Backend.search = text }
                Choice {
                    objectName: 'tasks-saved-views'
                    Layout.preferredWidth: Math.min(170, root.width * .22)
                    enabled: root.savedViews.length > 0
                    model: root.savedViews.map(v => v.name)
                    currentIndex: -1
                    displayText: root.savedViews.length ? (root.activeSavedViewName || 'Saved views') : 'No saved views'
                    Accessible.name: 'Saved views'
                    onActivated: index => { root.applySavedView(root.savedViews[index]); currentIndex = -1; }
                }
                SButton { text: (root.width > 470 ? 'Filter' : '') + ((Backend.branches.length || Backend.priority !== 'all' || Backend.status !== 'all' || Backend.showArchived) ? ' •' : ''); iconName: 'filter'; quiet: true; accessibleLabel: 'Filter tasks'; onClicked: taskFilterPopup.open() }
                STabButton { visible: Theme.pageTabs; text: 'List'; active: !Backend.board; onClicked: {Backend.board = false; Backend.remember();} }
                STabButton { visible: Theme.pageTabs; text: 'Board'; active: Backend.board; onClicked: {Backend.board = true; Backend.remember();} }
                SButton { text: 'Undo'; quiet: true; visible: Cal.canUndo(Backend, root.completionClock); onClicked: Cal.undoCompletion(Backend) }
                // Like Filter, Select keeps only its icon on a narrowed page, so the band fits its pane.
                SButton { objectName: 'tasks-select'; text: root.width > 470 ? 'Select' : ''; quiet: true; iconName: 'select'; accessibleLabel: 'Select tasks'; onClicked: root.beginSelection() }
            }
            RowLayout {
                anchors.fill: parent; spacing: 8; visible: root.selectionMode
                LabelText { Layout.fillWidth: true; text: 'Select · ' + root.selectedTaskIds.length; color: Theme.accentText; font.weight: Font.DemiBold }
                STabButton { text: 'List'; active: !Backend.board; onClicked: { Backend.board=false; Backend.remember(); } }
                STabButton { text: 'Board'; active: Backend.board; onClicked: { Backend.board=true; Backend.remember(); } }
                SButton { text: 'All'; accessibleLabel: 'Select all filtered tasks'; quiet: true; enabled: !root.batchBusy; onClicked: root.selectAll() }
                SButton { text: 'Actions'; enabled: root.selectedTaskIds.length > 0 && !root.batchBusy; busy: root.batchBusy; onClicked: batchPopup.open() }
                SButton { text: 'Done'; primary: true; enabled: !root.batchBusy; onClicked: root.endSelection() }
            }
        }
        // Quick add row
        RowLayout {
            Layout.fillWidth: true
            spacing: 8

            Field {
                id: quick
                Layout.fillWidth: true
                placeholderText: 'Add a task…'
                frameless: true
                onAccepted: root.addTask()
            }
            Choice {
                id: quickGoal
                Layout.preferredWidth: Math.min(176, root.width * .28)
                model: [{ id: '', title: 'No goal' }].concat(Backend.goals)
                textRole: 'title'
            }
            SButton {
                text: ''
                iconName: 'plus'
                accessibleLabel: 'Add task'
                primary: true
                enabled: quick.text.trim().length > 0
                onClicked: root.addTask()
            }
        }

        // List and board share one fixed frame, so switching never re-lays out the page.
        Item {
            id: switchFrame
            objectName: 'tasks-switch-frame'
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 0

        ListView {
            id: list
            SCollectionWheel { flickable: list }
            objectName: 'tasks-list'
            RetainedScroll { flickable: list; positionKey: 'Tasks/list' }
            visible: !Backend.board
            anchors.fill: parent
            clip: true
            spacing: 6
            model: tasksModel.model
            delegate: TaskCard {
                required property var modelData
                required property int index
                record: modelData
                width: list.width
                compact: true
                selectable: root.selectionMode
                selected: root.selectedTaskIds.indexOf(modelData.id) >= 0
                onSelectionToggled: shiftKey => root.toggleSelect(modelData, index, shiftKey, tasksModel.model)
                deletable: true
                onDeleteRequested: root.moveToTrash(modelData)
            }
            ScrollBar.vertical: SScrollBar {}
        }

        // Board View
        Flickable {
            id: board
            objectName: 'tasks-board'
            RetainedScroll { flickable: board; positionKey: 'Tasks/board-horizontal' }
            visible: Backend.board
            anchors.fill: parent
            clip: true
            contentWidth: Math.max(width, 696)
            contentHeight: height
            boundsBehavior: Flickable.StopAtBounds
            Row {
                spacing: 12
                height: board.height
                Repeater {
                    model: [
                        { state: 'todo', title: 'To do', color: Theme.muted },
                        { state: 'in_progress', title: 'In progress', color: Theme.accent },
                        { state: 'completed', title: 'Completed', color: Theme.green }
                    ]
                    Rectangle {
                        id: lane
                        objectName: "kanban-lane"
                        required property var modelData
                        property var tasks: tasksModel.model.filter(t => Cal.effectiveStatus(Backend, t) === modelData.state)
                        width: (board.contentWidth - 24) / 3
                        height: board.height
                        radius: Theme.cornerRadius
                        color: Theme.panel
                        border.color: drop.containsDrag ? Theme.accent : Theme.line
                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 10
                            spacing: 12
                            RowLayout {
                                Layout.fillWidth: true
                                Rectangle {
                                    Layout.preferredWidth: 7
                                    Layout.preferredHeight: 7
                                    radius: Theme.cornerRadius
                                    color: lane.modelData.color
                                }
                                LabelText {
                                    text: lane.modelData.title
                                    font.weight: Font.DemiBold
                                    Layout.fillWidth: true
                                }
                                LabelText {
                                    text: lane.tasks.length
                                    color: Theme.muted
                                }
                                SButton {
                                    text: '+'
                                    quiet: true
                                    onClicked: {
                                        Backend.create('task','','',Backend.today);
                                        Backend.edit('status', lane.modelData.state);
                                    }
                                }
                            }
                            ListView {
                                id: laneList
            SCollectionWheel { flickable: laneList }
                                RetainedScroll { flickable: laneList; positionKey: 'Tasks/board/' + lane.modelData.state }
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                clip: true
                                spacing: 8
                                model: lane.tasks
                                delegate: TaskCard {
                                    required property var modelData
                                    required property int index
                                    record: modelData
                                    width: laneList.width
                                    compact: true
                                    selectable: root.selectionMode
                                    selected: root.selectedTaskIds.indexOf(modelData.id) >= 0
                                    onSelectionToggled: shiftKey => root.toggleSelect(modelData, index, shiftKey, lane.tasks)
                                    deletable: true
                                    onDeleteRequested: root.moveToTrash(modelData)
                                }
                                ScrollBar.vertical: SScrollBar {}
                            }
                        }
                        DropArea {
                            id: drop
                            anchors.fill: parent
                            keys: ['task']
                            onDropped: event => {
                                Cal.completion(Backend, (event.source as CardDrag).record, lane.modelData.state);
                                event.acceptProposedAction();
                            }
                        }
                    }
                }
            }
            // A narrowed pane (detail and side panel open) leaves the lanes wider
            // than the page. The thumb shows only while it is reached for or the
            // board moves, so a resting board never carries a bar along its foot.
            ScrollBar.horizontal: SScrollBar {
                objectName: 'tasks-board-scrollbar'
                opacity: hovered || pressed || active ? 1 : 0
            }
        }
        }

        // Archive keeps tasks without deleting them; the Trash beside it holds the deleted ones.
        // Shown with archived tasks, as on Goals. Loaded by path: ui/qmldir does not list TrashView.qml.
        Loader {
            id: trashSection
            objectName: 'tasks-trash-section'
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(trashSection.contentHeight, Math.max(160, root.height * 0.4))
            readonly property real contentHeight: status === Loader.Ready && item ? (item as Item).implicitHeight : 0
            visible: Backend.showArchived
            active: Backend.showArchived && root.visible
            source: 'TrashView.qml'
            onLoaded: item.kinds = ['task']
        }
    }

    SPopup {
        id: taskFilterPopup
        parent: Overlay.overlay
        x: Math.max(8, (parent.width-width)/2); y: Math.max(8, (parent.height-height)/2)
        width: Math.min(520, parent.width-24)
        height: Math.min(filterContents.implicitHeight+32, parent.height-24)
        padding: 16; focus: true; modal: true
        background: Rectangle { color: Theme.panel; border.color: Theme.edge }
        contentItem: ScrollView {
            clip: true
            Column {
                id: filterContents; width: taskFilterPopup.availableWidth; spacing: 12
                LabelText { text: 'Filters'; font.pixelSize: 20; font.weight: Font.Medium }
        // Filter Bar (PLAN-07, PLAN-08)
        GridLayout {
            columns: 2
            width: parent.width
            columnSpacing: 8
            rowSpacing: 6

            // Category Filter
            Choice {
                id: catFilter
                Layout.fillWidth: true
                model: [{ id: '', title: 'All categories' }].concat(Backend.categories)
                textRole: 'title'
                onActivated: index => {
                    if (index === 0) {
                        Backend.branches = Backend.branches.filter(id => !Backend.categories.some(c => c.id === id));
                    } else {
                        var cid = model[index].id;
                        if (Backend.branches.indexOf(cid) < 0)
                            Backend.branches = Backend.branches.concat([cid]);
                    }
                    Backend.remember();
                }
            }

            // Goal Filter
            Choice {
                id: goalFilter
                Layout.fillWidth: true
                model: [{ id: '', title: 'All goals' }].concat(Backend.goals)
                textRole: 'title'
                onActivated: index => {
                    if (index === 0) {
                        Backend.branches = Backend.branches.filter(id => !Backend.goals.some(g => g.id === id));
                    } else {
                        var gid = model[index].id;
                        if (Backend.branches.indexOf(gid) < 0)
                            Backend.branches = Backend.branches.concat([gid]);
                    }
                    Backend.remember();
                }
            }

            // Priority Filter
            Choice {
                id: prioFilter
                Layout.fillWidth: true
                model: ['All priorities', 'High', 'Medium', 'Low']
                currentIndex: Backend.priority === 'high' ? 1 : Backend.priority === 'medium' ? 2 : Backend.priority === 'low' ? 3 : 0
                onActivated: index => {
                    Backend.priority = ['', 'high', 'medium', 'low'][index] || 'all';
                    Backend.remember();
                }
            }

            // Status Filter
            Choice {
                id: statusFilter
                Layout.fillWidth: true
                model: ['All statuses', 'To do', 'In progress', 'Completed']
                currentIndex: Backend.status === 'todo' ? 1 : Backend.status === 'in_progress' ? 2 : Backend.status === 'completed' ? 3 : 0
                onActivated: index => {
                    Backend.status = ['', 'todo', 'in_progress', 'completed'][index] || 'all';
                    Backend.remember();
                }
            }

            // Search
            SSearchField {
                id: searchInput
                Layout.fillWidth: true
                scopeName: 'tasks'
                text: Backend.search
                onTextEdited: Backend.search = text
            }

        }
        Flow {
            width: parent.width; spacing: 6
            Repeater {
                model: Backend.branches
                delegate: SButton {
                    required property string modelData
                    text: (Backend.get(modelData) ? Backend.get(modelData).title : modelData) + ' ×'
                    quiet: true; accessibleLabel: 'Remove filter ' + text
                    onClicked: Backend.toggleBranch(modelData)
                }
            }
        }
        SCheckBox { text: 'Show archived tasks'; checked: Backend.showArchived; onToggled: { Backend.showArchived = checked; Backend.remember(); } }
        SCheckBox { objectName: 'tasks-show-completed'; text: 'Show completed tasks'; checked: Backend.showCompleted; onToggled: { Backend.showCompleted = checked; Backend.remember(); } }
        SButton { text: 'Clear filters'; quiet: true; onClicked: Backend.clearFilters() }

        // Saved views: save the current filter by name, apply or remove each one here.
        LabelText { text: 'Saved views'; font.pixelSize: 15; font.weight: Font.Medium }
        RowLayout {
            width: parent.width; spacing: 8
            Field {
                id: viewNameField
                objectName: 'tasks-view-name'
                Layout.fillWidth: true
                placeholderText: 'Name this filter'
                onAccepted: if (text.trim()) { root.saveCurrentView(text); clear(); }
            }
            SButton {
                objectName: 'tasks-save-view'
                text: 'Save view'
                enabled: viewNameField.text.trim().length > 0
                onClicked: { root.saveCurrentView(viewNameField.text); viewNameField.clear(); }
            }
        }
        // Loaded by path: ui/qmldir lists its types and SavedViews.qml is not among them.
        Loader {
            id: savedViewsList
            width: parent.width
            source: 'SavedViews.qml'
            Binding { target: savedViewsList.item; property: 'views'; value: root.savedViews; when: savedViewsList.status === Loader.Ready }
            Binding { target: savedViewsList.item; property: 'activeName'; value: root.activeSavedViewName; when: savedViewsList.status === Loader.Ready }
            Connections {
                target: savedViewsList.item
                ignoreUnknownSignals: true
                function onApplyRequested(view) { root.applySavedView(view); }
                function onRemoveRequested(name) { Backend.deleteView(name); }
            }
        }

                SButton { text: 'Done'; onClicked: taskFilterPopup.close() }
            }
        }
    }
    SPopup {
        id: batchPopup
        parent: Overlay.overlay
        x: Math.max(8, (parent.width-width)/2); y: Math.max(8, (parent.height-height)/2)
        width: Math.min(460, parent.width-24)
        height: Math.min(batchControls.implicitHeight+32, parent.height-24)
        padding: 16; focus: true; modal: true
        background: Rectangle { color: Theme.panel; border.color: Theme.edge }
        contentItem: ScrollView {
            clip: true
            GridLayout {
                id: batchControls
                enabled: !root.batchBusy
                columns: 2
                width: parent.width
                columnSpacing: 8
                rowSpacing: 6

                LabelText {
                    text: root.selectedTaskIds.length + ' tasks selected'
                    font.weight: Font.DemiBold
                    font.pixelSize: 12
                    color: Theme.accent
                }

                SButton {
                    text: 'Select all'
                    quiet: true
                    onClicked: root.selectAll()
                }

                // Batch Status
                Choice {
                    Layout.preferredWidth: 120
                    model: ['Set status…', 'To do', 'In progress', 'Completed']
                    onActivated: index => {
                        if (index === 1) root.applyBatchStatus('todo');
                        else if (index === 2) root.applyBatchStatus('in_progress');
                        else if (index === 3) root.applyBatchStatus('completed');
                        currentIndex = 0;
                    }
                }

                // Batch Priority
                Choice {
                    Layout.preferredWidth: 120
                    model: ['Set priority…', 'Low', 'Medium', 'High']
                    onActivated: index => {
                        if (index === 1) root.applyBatchPriority('low');
                        else if (index === 2) root.applyBatchPriority('medium');
                        else if (index === 3) root.applyBatchPriority('high');
                        currentIndex = 0;
                    }
                }

                // Batch Goal
                Choice {
                    Layout.preferredWidth: 140
                    model: [{ id: '', title: 'Assign goal…' }].concat(Backend.goals)
                    textRole: 'title'
                    onActivated: index => {
                        if (index > 0) {
                            root.applyBatchGoal(model[index].id);
                        }
                        currentIndex = 0;
                    }
                }

                // Batch Owner
                Choice {
                    Layout.preferredWidth: 120
                    model: ['Set owner…', 'Human', 'Agent']
                    onActivated: index => {
                        if (index === 1) root.applyBatchOwner('human');
                        else if (index === 2) root.applyBatchOwner('agent');
                        currentIndex = 0;
                    }
                }

                SButton {
                    text: 'Archive'
                    quiet: true
                    onClicked: archiveReview.open()
                }

                Item { Layout.fillWidth: true }

                SButton {
                    text: 'Clear selection'
                    quiet: true
                    onClicked: root.deselectAll()
                }
            }

        }
    }

    // Deleting moves a task to the Trash at once; the notice offers Undo for ten seconds,
    // and the Trash (shown here and on Goals with archived items) keeps it for thirty days after that.
    property var deletedItem: null
    function moveToTrash(record) {
        if (!record || !record.id) return;
        var item = {kind: record.kind || 'task', id: record.id, title: record.title || 'Untitled task'};
        Backend.send('planner-trash', {kind: item.kind, id: item.id}, function(value, ok) {
            if (!ok) return;
            root.deletedItem = null;
            root.deletedItem = item;
            deletedNoticeTimer.restart();
        });
    }
    function undoDelete() {
        var item = deletedItem;
        if (!item) return;
        deletedItem = null;
        deletedNoticeTimer.stop();
        Backend.send('planner-trash-restore', {kind: item.kind, id: item.id});
    }
    Timer { id: deletedNoticeTimer; interval: 10000; onTriggered: root.deletedItem = null }
    NoticePopup {
        objectName: 'tasks-deleted-notice'
        scope: root
        error: false
        text: root.deletedItem ? '“' + root.deletedItem.title + '” moved to Trash.' : ''
        onDismissed: root.deletedItem = null
        SButton {
            id: tasksUndo
            objectName: 'tasks-deleted-undo'
            text: 'Undo'
            onClicked: root.undoDelete()
            // Beside the text, before the notice's own dismiss button.
            Component.onCompleted: { var row = parent; if (row && row.children.length > 2) row.children = [row.children[0], tasksUndo, row.children[1]]; }
        }
    }

    function addTask() {
        if (!quick.text.trim())
            return;
        var text = quick.text;
        var gid = quickGoal.model[quickGoal.currentIndex] ? quickGoal.model[quickGoal.currentIndex].id : '';
        Backend.send('save', {
            kind: 'task',
            title: text,
            goal_id: gid,
            scheduled_for: Backend.today
        }, (r, ok) => {
            if (ok && quick.text === text)
                quick.clear();
        });
    }
    StateView {
        anchors.centerIn: parent; width: Math.max(0, parent.width-40); height: Math.max(0, parent.height-160)
        visible: Backend.tasks.length === 0
        title: Backend.showArchived ? 'No archived tasks' : 'No matching tasks'
        detail: 'Clear the filters or add a task to begin.'
    }
}
