pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Presentation.js" as Presentation
import "Workplane.js" as Workplane

Item {
    id: root
    objectName: 'goals-view'
    property var categories: Backend.records.filter(r => r.kind === 'category' && (Backend.showArchived ? r.archived : !r.archived)).sort((a,b) => a.order-b.order)
    // The selection is kept in the preferences, so the side panel's category rows and this page share it.
    readonly property var selectedCategories: Presentation.categorySelection(Backend.prefs.goalsCategories, Backend.prefs.goalsCategory)
    property bool contextReady: false
    property string query: Backend.prefs.goalsQuery || ''
    property var menuRecord: null
    property var shownGoals: goalsModel.model
    onQueryChanged: if (contextReady) Backend.setting('goalsQuery', query)
    Component.onCompleted: contextReady = true
    function toggleCategory(id) { Backend.setting('goalsCategories', Presentation.toggled(selectedCategories, id)); }
    // A goal or category opened here shows its details inside its own pane over the grid,
    // never in the record pane beside it, so the columns and the scroll stay where they were.
    readonly property string inPlaceRoute: Backend.routeKey + '#in-place'
    readonly property bool detailInPlace: root.visible && Backend.page === 'Goals' && Backend.detailsOpen
        && Backend.detailRoute === inPlaceRoute && !!Backend.activeDraft && ['goal','category'].indexOf(Backend.activeDraft.kind) >= 0
    function adoptDetails() {
        if (!root.visible || Backend.page !== 'Goals' || !Backend.detailsOpen || Backend.detailRoute !== Backend.routeKey) return;
        if (!Backend.activeDraft || ['goal','category'].indexOf(Backend.activeDraft.kind) < 0) return;
        Backend.detailRoute = inPlaceRoute;
        Backend.compactPane = 'work';
    }
    onVisibleChanged: adoptDetails()
    Connections {
        target: Backend
        function onCategoryActionsRequested(record, item) { root.openActions(record, item); }
        function onDetailRouteChanged() { root.adoptDetails(); }
        function onDetailsOpenChanged() { root.adoptDetails(); }
        function onActiveKeyChanged() { root.adoptDetails(); }
    }
    function openActions(record, item) {
        menuRecord = record;
        actions.trigger = item;
        var p = item.mapToItem(Overlay.overlay, 0, 0);
        var placement = Workplane.popupPosition({x:p.x,y:p.y,width:item.width,height:item.height},
            {width:actions.width,height:actions.height},{width:Overlay.overlay.width,height:Overlay.overlay.height});
        actions.x=placement.x; actions.y=placement.y;
        actions.open();
    }
    RetainedModel {
        id: goalsModel
        active: root.visible
        sourceModel: Backend.records.filter(r => r.kind === 'goal' && (Backend.showArchived ? r.hidden : !r.hidden) && (!root.selectedCategories.length || root.selectedCategories.indexOf(r.category_id) >= 0) && Presentation.matchesText(r, root.query)).sort((a,b) => a.order-b.order)
    }
    SplitView {
        id: goalSplit
        anchors.fill: parent
        orientation: Qt.Horizontal
        handle: SSplitHandle { }
            // The categories column: the same list, rows and colours as the former side panel.
            SidePanel {
                objectName: 'categories-pane'
                showEntries: false
                visible: Theme.pageTabs
                SplitView.minimumWidth: 180
                SplitView.preferredWidth: Theme.sidePanelWidth
                SplitView.maximumWidth: Math.max(180, Math.min(320, root.width * 0.40))
            }
        ColumnLayout {
            id: goalsPane
            objectName: 'goals-pane'
            SplitView.fillWidth: true
            SplitView.minimumWidth: root.width < 660 ? 180 : 240
            spacing: 12
        GridLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: goalsPane.width < 400 ? 84 : 40
            columns: goalsPane.width < 400 ? 1 : 2
            columnSpacing: 8; rowSpacing: 4
            SSearchField { Layout.fillWidth: true; Layout.minimumWidth: 40; scopeName: 'goals'; frameless: true; text: root.query; onTextEdited: root.query=text }
            RowLayout {
                Layout.alignment: Qt.AlignRight
                spacing: 4
            STabButton { visible: Theme.pageTabs; text: 'Cards'; active: !Backend.goalList; onClicked: {Backend.goalList=false;Backend.remember();} }
            STabButton { visible: Theme.pageTabs; text: 'List'; active: Backend.goalList; onClicked: {Backend.goalList=true;Backend.remember();} }
            STabButton { visible: Theme.pageTabs; text: 'Archive'; active: Backend.showArchived; onClicked: {Backend.showArchived = !Backend.showArchived; Backend.remember();} }
            SButton { iconName:'plus'; text: root.width>560?'Goal':''; accessibleLabel:'New goal'; primary:true; onClicked:Backend.create('goal','',root.selectedCategories.length===1?root.selectedCategories[0]:'') }
        
            }
        }
            Flickable {
                id:goals;objectName:'goals-list';Layout.fillWidth:true;Layout.fillHeight:true
            SCollectionWheel { flickable: goals }
                clip:true;contentWidth:width;contentHeight:cards.implicitHeight;boundsBehavior:Flickable.StopAtBounds
                RetainedScroll {flickable:goals;positionKey:'Goals/goals'}
                ScrollBar.vertical:SScrollBar {}
                GridLayout {
                    id:cards;width:Math.max(0,goals.width);columns:Backend.goalList?1:Math.max(1,Math.floor(goals.width/266));columnSpacing:12;rowSpacing:12
                    Repeater {
                        id:goalCards
                        model:root.shownGoals
                        Rectangle {
                            id:goal;required property var modelData
                            objectName:'goal-card'
                            Layout.fillWidth:true;Layout.minimumWidth:0;Layout.preferredWidth:Math.max(0,(cards.width-(cards.columns-1)*12)/cards.columns);Layout.preferredHeight:Backend.goalList?134:194
                            color:Backend.selectedId===modelData.id?Theme.selection:goalDrag.containsMouse?Theme.raised:Theme.panel
                            border.color:activeFocus?Theme.accent:Backend.selectedId===modelData.id?Theme.edge:'transparent';border.width:1;radius:Theme.cornerRadius
                            activeFocusOnTab:true;Accessible.role:Accessible.ListItem;Accessible.name:modelData.title;Accessible.selected:Backend.selectedId===modelData.id
                            Keys.onReturnPressed:Backend.select(modelData.id)
                            Keys.onSpacePressed:Backend.select(modelData.id)
                            Keys.onPressed:event=>{if(event.key===Qt.Key_Menu || (event.key===Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier))) {root.openActions(modelData,goal);event.accepted=true;}}
                            CardDrag {id:goalDrag;card:goal;record:goal.modelData;keys:['goal'];onActivated:{Backend.select(goal.modelData.id);goal.forceActiveFocus();}onOpened:{Backend.select(goal.modelData.id);Backend.detailFull=true;}}
                            DropArea {
                                id:goalDrop;anchors.fill:parent;keys:['goal']
                                property bool after:Backend.goalList?drag.y>height/2:drag.x>width/2
                                onDropped:event=>{Backend.placeRecord((event.source as CardDrag).record,goal.modelData,after);event.acceptProposedAction();}
                                Rectangle {color:Theme.accent;visible:goalDrop.containsDrag;width:Backend.goalList?parent.width:2;height:Backend.goalList?2:parent.height;x:!Backend.goalList&&goalDrop.after?parent.width-2:0;y:Backend.goalList&&goalDrop.after?parent.height-2:0}
                            }
                            ColumnLayout {
                                anchors.fill:parent;anchors.margins:18;spacing:8
                                RowLayout {
                                    Layout.fillWidth:true
                                    Rectangle {Layout.preferredWidth:5;Layout.preferredHeight:5;color:goal.modelData.color||Theme.accent}
                                    LabelText {Layout.fillWidth:true;text:goal.modelData.category_title||'Unassigned';color:Theme.muted;font.pixelSize:12}
                                    PrivateMarker {visible:!Backend.privateMode&&Backend.isMarkedPrivate(goal.modelData.id)}
                                    SButton {id:goalMenu;iconName:'more';accessibleLabel:'Actions for '+goal.modelData.title;quiet:true;opacity:goalDrag.containsMouse||hovered||activeFocus||goal.activeFocus?1:0;onClicked:root.openActions(goal.modelData,goalMenu)}
                                }
                                LabelText {Layout.fillWidth:true;text:goal.modelData.title;font.pixelSize:18;font.weight:Font.DemiBold;wrapMode:Text.Wrap;maximumLineCount:2}
                                LabelText {Layout.fillWidth:true;Layout.fillHeight:true;visible:!Backend.goalList;text:goal.modelData.body||'';font.pixelSize:13;color:Theme.muted;wrapMode:Text.Wrap;maximumLineCount:2}
                                RowLayout {
                                    Layout.fillWidth:true
                                    LabelText {Layout.fillWidth:true;text:goal.modelData.completed_count !== undefined && goal.modelData.task_count !== undefined ? goal.modelData.completed_count+' of '+goal.modelData.task_count+' tasks complete' : 'Task count unavailable';font.pixelSize:12;color:Theme.muted}
                                    SButton {iconName:'plus';accessibleLabel:'New task in '+goal.modelData.title;quiet:true;opacity:goalDrag.containsMouse||hovered||activeFocus||goal.activeFocus?1:0;onClicked:Backend.create('task',goal.modelData.id,goal.modelData.category_id,Backend.today)}
                                }
                                Rectangle {Layout.fillWidth:true;Layout.preferredHeight:2;color:Theme.line;visible:!Backend.goalList && goal.modelData.task_count !== undefined && goal.modelData.completed_count !== undefined;Rectangle {height:parent.height;width:parent.width*Math.max(0,Math.min(1,goal.modelData.task_count?goal.modelData.completed_count/goal.modelData.task_count:0));color:goal.modelData.color||Theme.accent}}
                            }
                        }
                    }
                }
                StateView {anchors.fill:parent;visible:root.shownGoals.length===0;title:root.query?'No matching goals':'No goals in this selection';detail:root.query?'Try a different search.':'Choose other categories or create a goal.'}
            }
            // Archive keeps items without deleting them; the Trash beside it holds what was deleted.
            // Loaded by path: ui/qmldir lists its types and TrashView.qml is not among them.
            Loader {
                id: trashSection
                objectName: 'goals-trash-section'
                Layout.fillWidth: true
                Layout.preferredHeight: Math.min(trashSection.contentHeight, Math.max(160, goalsPane.height * 0.5))
                // The section's own height, read through the loaded item.
                readonly property real contentHeight: status === Loader.Ready && item ? (item as Item).implicitHeight : 0
                visible: Backend.showArchived
                active: Backend.showArchived && root.visible
                source: 'TrashView.qml'
            }
        }
    }
    Rectangle {
        id: goalDetail
        objectName: 'goal-detail'
        // The opened goal's card, found by id; a category or a new goal has none and opens at the grid's corner.
        readonly property Item card: {
            var id = root.detailInPlace ? Backend.activeDraft.id : '';
            for (var i = 0; id && i < goalCards.count; i++)
                if (root.shownGoals[i] && root.shownGoals[i].id === id) return goalCards.itemAt(i);
            return null;
        }
        readonly property rect cardRect: {
            void(goals.contentY); void(cards.width); void(cards.implicitHeight);
            if (!card) return Qt.rect(0, 0, 0, 0);
            void(card.x); void(card.y);
            var p = card.mapToItem(goals, 0, 0);
            return Qt.rect(p.x, p.y, card.width, card.height);
        }
        readonly property point origin: { void(goals.y); void(goalsPane.x); void(root.width); return goals.mapToItem(root, 0, 0); }
        readonly property real fullHeight: Math.min(goals.height, 640)
        visible: root.detailInPlace
        z: 2
        width: Backend.detailFull ? goals.width : Math.min(goals.width, Math.max(cardRect.width, 620))
        height: Backend.detailFull ? goals.height : fullHeight
        x: origin.x + Math.max(0, Math.min(cardRect.x, goals.width - width))
        y: origin.y + Math.max(0, Math.min(cardRect.y, goals.height - height))
        color: Theme.base
        border.color: Theme.edge; border.width: 1; radius: Theme.cornerRadius
        Rectangle {
            x: 1; y: 1; width: parent.width - 2; height: 2
            color: (root.detailInPlace && Backend.activeDraft.color) || Theme.accent
        }
        DetailPane {
            anchors.fill: parent; anchors.margins: 1; anchors.topMargin: 3
            visible: root.detailInPlace
        }
    }
    // Deleting moves a goal or a category to the Trash at once; the notice offers Undo for ten seconds,
    // and the Trash in Archive keeps it for thirty days after that.
    property var deletedItem: null
    function moveToTrash(record) {
        if (!record || !record.id) return;
        var item = {kind: record.kind, id: record.id, title: record.title || (record.kind === 'category' ? 'Untitled category' : 'Untitled goal')};
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
        objectName: 'goals-deleted-notice'
        scope: root
        error: false
        text: !root.deletedItem ? '' : root.deletedItem.kind === 'category'
            ? '“' + root.deletedItem.title + '” and its goals moved to Trash.'
            : '“' + root.deletedItem.title + '” moved to Trash.'
        onDismissed: root.deletedItem = null
        SButton {
            id: goalsUndo
            objectName: 'goals-deleted-undo'
            text: 'Undo'
            onClicked: root.undoDelete()
            // Beside the text, before the notice's own dismiss button.
            Component.onCompleted: { var row = parent; if (row && row.children.length > 2) row.children = [row.children[0], goalsUndo, row.children[1]]; }
        }
    }
    SPopup {
        id:actions;parent:Overlay.overlay;width:228;padding:6
        contentItem:ColumnLayout {spacing:2
            MenuAction {text:'Edit';iconName:'edit';onClicked:{var id=root.menuRecord.id;actions.close();Backend.select(id);}}
            // The engine moves the goal, or the category with its goals, to the Trash; an error shows the engine's own words.
            MenuAction {text:'Delete';visible:!!root.menuRecord&&['goal','category'].indexOf(root.menuRecord.kind)>=0;onClicked:{var r=root.menuRecord;actions.close();root.moveToTrash(r);}}
            MenuAction {text:'New task';iconName:'plus';visible:root.menuRecord&&root.menuRecord.kind==='goal';onClicked:{var r=root.menuRecord;actions.close();Backend.create('task',r.id,r.category_id,Backend.today);}}
            MenuAction {text:root.menuRecord&&root.menuRecord.archived?'Restore from archive':'Archive';visible:!!root.menuRecord&&root.menuRecord.kind==='goal';onClicked:{var r=root.menuRecord;actions.close();Backend.mutate(r,{archived:!r.archived});}}
            MenuAction {text:'Archive category…';visible:!!root.menuRecord&&root.menuRecord.kind==='category'&&!root.menuRecord.archived;onClicked:{actions.close();categoryReview.open();}}
            MenuAction {text:'Restore category';visible:!!root.menuRecord&&root.menuRecord.kind==='category'&&root.menuRecord.archived;onClicked:{var r=root.menuRecord;actions.close();Backend.mutate(r,{archived:false});}}
            MenuAction {readonly property bool marked:!!root.menuRecord&&Backend.isMarkedPrivate(root.menuRecord.id);text:marked?'Unmark private':'Mark private';iconName:'private';visible:!!root.menuRecord&&!!root.menuRecord.id;onClicked:{var id=root.menuRecord.id,on=!marked;actions.close();Backend.setMarkedPrivate(id,on);}}
        }
    }
    SDialog {
        id: categoryReview
        parent: Overlay.overlay
        anchors.centerIn: parent
        width: Math.min(480, parent.width - 32)
        title: 'Archive category'
        modal: true
        focus: true
        property string errorText: ''
        property var pendingArchive: null
        // Finish popup focus restoration while its category delegate still exists.
        // Only then may the owner archive remove that delegate from the list.
        onClosed: {
            var current = pendingArchive;
            pendingArchive = null;
            if (!current) return;
            Qt.callLater(function() {
                Backend.mutate(current, {archived:true}, function(value, ok) {
                    if (ok) Backend.message('Category archived');
                    else {
                        categoryReview.open();
                        categoryReview.errorText = 'Category changed or could not be archived. Review it and retry.';
                    }
                });
            });
        }
        readonly property var affectedGoals: Backend.records.filter(r => r.kind === 'goal' && root.menuRecord && r.category_id === root.menuRecord.id)
        readonly property var affectedTasks: Backend.records.filter(r => r.kind === 'task' && root.menuRecord && (r.category_id === root.menuRecord.id || categoryReview.affectedGoals.some(g => g.id === r.goal_id)))
        onAboutToShow: errorText = ''
        contentItem: ColumnLayout {
            spacing: 14
            LabelText { Layout.fillWidth: true; wrapMode: Text.Wrap; text: root.menuRecord ? '“' + root.menuRecord.title + '” has ' + categoryReview.affectedGoals.length + ' goals and ' + categoryReview.affectedTasks.length + ' tasks.' : '' }
            LabelText { Layout.fillWidth: true; wrapMode: Text.Wrap; text: 'Archive this category and hide its goals and tasks from normal views. Linked notes remain accessible. Restore the category from Archive; no goal or task is erased.' }
            LabelText { Layout.fillWidth: true; visible: !!categoryReview.errorText; color: Theme.dangerText; text: categoryReview.errorText; wrapMode: Text.Wrap }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                SButton { text: 'Cancel'; onClicked: categoryReview.requestDismiss() }
                SButton { text: 'Archive category'; destructive: true; onClicked: {
                    var current = root.menuRecord ? Backend.get(root.menuRecord.id) : null;
                    if (!current || current.kind !== 'category') { categoryReview.errorText = 'Category changed. Reopen its actions.'; return; }
                    categoryReview.pendingArchive = current;
                    categoryReview.close();
                } }
            }
        }
    }
}
