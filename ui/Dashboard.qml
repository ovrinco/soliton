pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "CalendarMath.js" as Cal

Item {
    id: root
    objectName: 'dashboard-page'
    property real completionClock: Date.now()
    Timer { interval: 250; repeat: true; running: root.visible && !!(Backend.prefs.lastTaskCompletion || {}).undoUntil && (Backend.prefs.lastTaskCompletion || {}).undoUntil > root.completionClock; onTriggered: root.completionClock = Date.now() }
    property string today: Backend.today
    property var active: Backend.records.filter(r => r.kind === 'task' && !r.hidden && Cal.showTask(Backend, r, root.completionClock) && Backend.matches(r))
    property var todayTasks: Backend.todayEvents.filter(r => Cal.showTask(Backend, r, root.completionClock) && (r.date === today || (r.event_kind === 'timed' && r.date < today && r.last_date >= today))).filter(r => Backend.matches(r)).filter((r, i, all) => all.findIndex(x => x.id === r.id && x.occurrence === r.occurrence) === i)
    property var overdue: active.filter(r => r.due_on && r.due_on < today)
    readonly property var timelineTasks: {
        var seen = {}, result = [];
        function add(records) { records.forEach(function(r) { var key=r.id+':'+(r.occurrence||'');if(!seen[key]) { seen[key]=true;result.push(r); } }); }
        add(root.overdue); add(root.todayTasks); add(root.active.filter(r=>r.status==='in_progress'));
        add(root.active.filter(r=>!result.some(e=>e.id===r.id)));
        return result;
    }
    RetainedModel { id: timelineModel; active: root.visible && root.activeMode === 'Timeline'; sourceModel: root.timelineTasks }
    // The work list is updated row by row, never replaced, so a tick keeps the
    // list's scroll position. While the overview stays open the rows keep the
    // order they were shown in, and a task completed here keeps its row; a new
    // visit or a filter change re-sorts them.
    ListModel { id: workRows }
    property string workOrderScope: ''
    function workKey(r) { return r.id + ':' + (r.occurrence || ''); }
    function syncWork() {
        var source = timelineModel.model || [], byKey = {}, order = [];
        source.forEach(r => byKey[workKey(r)] = r);
        var scope = JSON.stringify([Backend.branches, Backend.priority, Backend.status, Backend.search, Backend.showCompleted, Backend.showArchived]);
        if (scope === workOrderScope) {
            for (var i = 0; i < workRows.count; i++) {
                var kept = workRows.get(i).key;
                if (!byKey[kept]) {
                    var shown = JSON.parse(workRows.get(i).recordJson), fresh = Backend.get(shown.id);
                    if (!fresh || fresh.hidden || Cal.effectiveStatus(Backend, fresh) !== 'completed') continue;
                    byKey[kept] = shown.occurrence ? Object.assign({}, shown, {status: 'completed'}) : fresh;
                }
                order.push(kept);
            }
        }
        workOrderScope = scope;
        source.forEach(r => { var key = workKey(r); if (order.indexOf(key) < 0) order.push(key); });
        order.forEach((key, index) => {
            var row = {key: key, recordJson: JSON.stringify(byKey[key])};
            if (index >= workRows.count) workRows.append(row);
            else if (workRows.get(index).key !== row.key || workRows.get(index).recordJson !== row.recordJson) workRows.set(index, row);
        });
        if (workRows.count > order.length) workRows.remove(order.length, workRows.count - order.length);
    }
    Connections { target: timelineModel; function onModelChanged() { root.syncWork(); } }
    onVisibleChanged: if (!visible) workOrderScope = ''
    Component.onCompleted: syncWork()
    // Saved views are made on Tasks; the Dashboard only invokes one. Its
    // filter is the shared engine scope, so every view below follows it.
    readonly property var savedViews: Backend.prefs.savedViews || []
    function sameScope(v) {
        return JSON.stringify((v.branches || []).slice().sort()) === JSON.stringify(Backend.branches.slice().sort())
            && (v.priority || 'all') === Backend.priority && (v.status || 'all') === Backend.status && (v.search || '') === Backend.search;
    }
    readonly property bool unfiltered: !Backend.branches.length && Backend.priority === 'all' && Backend.status === 'all' && !Backend.search
    readonly property int savedViewIndex: savedViews.findIndex(v => sameScope(v))
    function goalMatches(g) {
        return (!Backend.branches.length || Backend.branchMatches(g, []))
            && (!Backend.search || String(g.title || '').toLowerCase().indexOf(Backend.search.toLowerCase()) >= 0);
    }
    property string activeMode: Backend.view && ['Timeline', 'Day', 'Week', 'Month', 'Year'].indexOf(Backend.view) >= 0 ? Backend.view : 'Timeline'
    // The tabs are views of one page, so the detail pane keeps its state
    // across them: navigate restores each route's own details, and a tab
    // without them threw the page from beside the pane to full width.
    // The record shown and its draft carry over unchanged.
    function switchTab(mode) {
        var open = Backend.hasRouteDetails, id = Backend.selectedId, key = Backend.activeKey, full = Backend.detailFull;
        Backend.navigate('Dashboard', mode);
        if (['Day','Week','Month','Year'].indexOf(mode)>=0) Backend.calendarMode=mode;
        if (open && (!Backend.hasRouteDetails || Backend.activeKey !== key)) {
            Backend.selectedId = id; Backend.activeKey = key;
            Backend.detailsOpen = true; Backend.detailRoute = Backend.routeKey;
            Backend.detailFull = full; Backend.compactPane = 'detail';
            Backend.remember();
        } else if (!open && Backend.hasRouteDetails) {
            Backend.closeDetails();
        }
    }
    // A short strip centres the active tab, so the tabs either side of it
    // stay in view and in reach; a full strip rests at its start.
    function revealActiveTab() {
        var view = tabStrip.children[0], x = 0;
        for (var i = 0; i < tabButtons.count; i++) {
            var tab = tabButtons.itemAt(i) as STabButton;
            if (tab && tab.active) x = tab.mapToItem(view.contentItem, 0, 0).x + (tab.width - view.width) / 2;
        }
        view.contentX = Math.max(0, Math.min(x, view.contentWidth - view.width));
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 14

        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 40
            // Every control here shrinks within a stated range. A control left at
            // its preferred width fixes the row's minimum, and the page's one
            // column then takes that minimum: with details open the view stack
            // ran 130 pixels past the page. The strip scrolls once it is short.
            STabBar {
                id: tabStrip
                visible: Theme.pageTabs
                Layout.fillWidth: true
                Layout.minimumWidth: Math.min(implicitWidth, 88)
                Layout.preferredWidth: Math.min(root.width * .72, 550)
                Layout.maximumWidth: Layout.preferredWidth
                onWidthChanged: Qt.callLater(root.revealActiveTab)
                Repeater {
                    id: tabButtons
                    model: ['Timeline', 'Day', 'Week', 'Month', 'Year']
                    delegate: STabButton {
                        required property string modelData
                        Layout.preferredWidth: 88
                        onActiveChanged: if (active) Qt.callLater(root.revealActiveTab)
                        text: modelData === 'Timeline' ? 'Overview' : modelData; active: root.activeMode === modelData
                        onClicked: root.switchTab(modelData)
                    }
                }
            }
            Item { Layout.fillWidth: true }
            SButton { text: 'Undo'; quiet: true; visible: Cal.canUndo(Backend, root.completionClock); onClicked: Cal.undoCompletion(Backend) }
            SCheckBox { text: root.width > 820 ? 'Show completed' : ''; Accessible.name: 'Show completed'; checked: Backend.showCompleted; onToggled: { Backend.showCompleted = checked; Backend.remember(); } }
            Choice {
                objectName: 'dashboard-saved-view'
                Layout.fillWidth: true
                Layout.minimumWidth: 96
                Layout.preferredWidth: 200
                Layout.maximumWidth: 200
                popup.implicitHeight: Math.min(count * Theme.denseRowHeight + 8, 440)
                model: root.savedViews.length ? ['All tasks'].concat(root.savedViews.map(v => v.name)) : ['No saved views']
                displayText: !root.savedViews.length ? 'No saved views' : root.savedViewIndex >= 0 ? root.savedViews[root.savedViewIndex].name : root.unfiltered ? 'All tasks' : 'Unsaved filter'
                currentIndex: root.savedViewIndex >= 0 ? root.savedViewIndex + 1 : 0
                Accessible.name: 'Saved view'
                onActivated: index => {
                    if (!root.savedViews.length) return;
                    if (index === 0) Backend.clearFilters();
                    else Backend.applyScope(root.savedViews[index - 1]);
                }
            }
            SButton { iconName:'plus'; text:root.width>720?'Task':''; primary:true; accessibleLabel:'New task'; onClicked:Backend.create('task','','',Backend.today) }
        }

        // View Stack: Timeline / Calendar
        StackLayout {
            objectName: 'dashboard-views'
            Layout.minimumWidth: 0; Layout.minimumHeight: 0
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: root.activeMode === 'Timeline' ? 0 : 1

            // Tab 0: Timeline Overview
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 16

                // One column: the work list fills the page and the active
                // goals follow below as a band of rows in columns, so neither
                // is squeezed into a pane beside the other. The quiet handle
                // between them still resizes the band.
                SplitView {
                    id: dashboardSplit
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    orientation: Qt.Vertical
                    handle: SSplitHandle { horizontal: false }
                    onResizingChanged: if (!resizing) goalsHeightSave.restart()
                    Timer { id: goalsHeightSave; interval: 150; onTriggered: {
                        var value = Math.round(activeGoalsPane.height);
                        if (value > 0 && Backend.prefs.dashboardGoalsHeight !== value)
                            Backend.setting('dashboardGoalsHeight', value);
                    } }

                    ColumnLayout {
                        SplitView.fillWidth: true; SplitView.fillHeight: true
                        SplitView.minimumHeight: 120
                        spacing: 10
                        RowLayout {
                            Layout.fillWidth: true
                            LabelText {
                                text: 'Your work'
                                font.pixelSize: 14; font.weight: Font.Medium
                                Layout.fillWidth: true
                            }
                        }
                        ListView {
                            id: next
                            SCollectionWheel { flickable: next }
                            RetainedScroll { flickable: next; positionKey: 'Dashboard/next' }
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
                            spacing: 8
                            model: workRows
                            delegate: TaskCard {
                                required property string recordJson
                                record: JSON.parse(recordJson)
                                width: next.width
                                compact: true
                            }
                            StateView { anchors.fill: parent; visible: next.count === 0; title: 'No tasks in this view' }
                            ScrollBar.vertical: SScrollBar {}
                        }
                    }

                    ColumnLayout {
                        id: activeGoalsPane
                        // Two rows of goals by default; the rest scroll in the band.
                        readonly property real twoRows: goalsHeading.implicitHeight + spacing + 2 * goalList.cellHeight + 16
                        SplitView.minimumHeight: goalsHeading.implicitHeight + spacing + goalList.cellHeight + 16
                        SplitView.preferredHeight: Number(Backend.prefs.dashboardGoalsHeight) || twoRows
                        SplitView.maximumHeight: Math.max(SplitView.minimumHeight, root.height * 0.6)
                        spacing: 10
                        LabelText {
                            id: goalsHeading
                            Layout.topMargin: 16
                            text: 'Active goals'
                            font.pixelSize: 14; font.weight: Font.Medium
                        }
                        GridView {
                            id: goalList
                            objectName: 'dashboard-active-goals'
                            SCollectionWheel { flickable: goalList }
                            RetainedScroll { flickable: goalList; positionKey: 'Dashboard/goals' }
                            readonly property int columns: Math.max(1, Math.floor(width / 320))
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
                            cellWidth: width / columns
                            cellHeight: 62
                            model: Backend.goals.filter(g => g.status === 'active' && root.goalMatches(g))
                            delegate: Item {
                                id: goalCell
                                required property var modelData
                                width: goalList.cellWidth
                                height: goalList.cellHeight
                                CollectionRow {
                                    anchors.fill: parent
                                    anchors.rightMargin: 16
                                    heading: goalCell.modelData.title
                                    metadata: ((Backend.categories.find(c => c.id === goalCell.modelData.category_id) || {}).title || 'Unassigned') + (goalCell.modelData.completed_count !== undefined && goalCell.modelData.task_count !== undefined ? ' · '+goalCell.modelData.completed_count+'/'+goalCell.modelData.task_count : '')
                                    symbol: 'goals'
                                    summary: ''
                                    selected: Backend.selectedId === goalCell.modelData.id
                                    onClicked: Backend.select(goalCell.modelData.id)
                                }
                            }
                            StateView { anchors.fill: parent; visible: goalList.count === 0; title: 'No goals in this view' }
                            ScrollBar.vertical: SScrollBar {}
                        }
                    }
                }
            }

            // Tab 1: Calendar View (Day / Week / Month / Year)
            CalendarView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                embedded: true
            }
        }
    }
}
