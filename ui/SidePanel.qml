import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Presentation.js" as Presentation
import "Routes.js" as Routes

// Beside the activity rail: the page's own navigation, as a list.
Rectangle {
    id: panel
    objectName: 'soliton-side-panel'
    color: Theme.panel
    implicitWidth: Theme.sidePanelWidth
    // In the Goals page's own column only the category list shows.
    property bool showEntries: true

    // One row of the filter list: the look of the view rows, with an optional
    // category's colour bar at its left edge and the item's quiet menu.
    component PanelRow: AbstractButton {
        id: row
        property bool active: false
        property bool twoLines: false
        property color barColor: Theme.accent
        property bool hasBar: barColor !== Theme.accent
        property bool marked: false
        property bool lit: hovered
        property int textInset: 16
        signal menuRequested(Item item)
        Layout.fillWidth: true
        implicitHeight: twoLines ? Math.max(32, label.implicitHeight + 12) : 32
        hoverEnabled: true
        focusPolicy: Qt.StrongFocus
        Keys.onPressed: event => {
            if (event.key === Qt.Key_Menu || (event.key === Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier))) { row.menuRequested(row); event.accepted = true; }
        }
        TapHandler { acceptedButtons: Qt.RightButton; onTapped: row.menuRequested(row) }
        contentItem: RowLayout {
            spacing: 6
            Text {
                id: label
                Layout.fillWidth: true
                leftPadding: row.textInset
                text: row.text
                font.family: Theme.font; font.pixelSize: 14
                font.weight: row.active ? Font.Medium : Font.Normal
                color: row.active || row.lit ? Theme.text : Theme.muted
                verticalAlignment: Text.AlignVCenter
                wrapMode: row.twoLines ? Text.Wrap : Text.NoWrap
                maximumLineCount: row.twoLines ? 2 : 1
                elide: Text.ElideRight
            }
            PrivateMarker { visible: row.marked }
            SButton {
                visible: row.twoLines
                iconName: 'more'; quiet: true
                accessibleLabel: 'Actions for ' + row.text
                opacity: row.lit || hovered || activeFocus || row.activeFocus ? 1 : 0
                onClicked: row.menuRequested(this)
            }
        }
        background: Rectangle {
            color: row.active ? Theme.raised : row.lit ? Theme.hover : 'transparent'
            Rectangle { objectName: 'category-colour'; width: 2; height: parent.height; color: row.barColor; visible: row.hasBar }
            Rectangle { anchors.fill: parent; color: 'transparent'; border.width: 2; border.color: Theme.accent; visible: row.visualFocus }
        }
        Accessible.role: Accessible.PageTab
        Accessible.name: text
        Accessible.selected: active
    }

    readonly property var entries: {
        // The Dashboard's horizons are its own tab bar; the panel lists no second copy.
        // Hearth lists the family's modules that have landed.
        if (Backend.page === 'Hearth')
            return Backend.hearthViews.map(v => ({ label: v, active: Backend.view === v, run: () => Backend.navigate('Hearth', v) }));
        if (Backend.page === 'Tasks')
            return [
                { label: 'List', active: !Backend.board, run: () => { Backend.board = false; Backend.remember(); } },
                { label: 'Board', active: Backend.board, run: () => { Backend.board = true; Backend.remember(); } }
            ];
        if (Backend.page === 'Goals')
            return [
                { label: 'Cards', active: !Backend.goalList, run: () => { Backend.navigate('Goals'); Backend.goalList = false; Backend.remember(); } },
                { label: 'List', active: Backend.goalList, run: () => { Backend.navigate('Goals'); Backend.goalList = true; Backend.remember(); } },
                { label: 'Archive', active: Backend.showArchived, run: () => { Backend.navigate('Goals'); Backend.showArchived = !Backend.showArchived; Backend.remember(); } }
            ];
        if (Backend.page === 'Notes')
            return [{ label: 'Notes', active: true, run: () => Backend.navigate('Notes', 'Notes') }];
        // Pages whose views are routes: one row per view, in the order of the page's former tabs.
        var views = Routes.pageViews(Backend.page);
        if (views.length)
            return views.map(v => ({ label: v, active: panel.routeActive(v), run: () => panel.openView(v) }));
        return [];
    }
    function routeActive(v) {
        if (Backend.page === 'Strategy') {
            if (Backend.view === 'History') return Presentation.historyTab(Backend.prefs.historyKind) === v;
            if (Routes.pageViews('Strategy').indexOf(Backend.view) >= 0) return Backend.view === v;
            return v === 'Overview';
        }
        if (Backend.page === 'Settings')
            return (Backend.view === 'Plugins & Skills' ? 'Plugins' : Backend.view) === v;
        return Backend.view === v;
    }
    function openView(v) {
        if (Backend.page === 'Strategy') Backend.strategyTab = v;
        Backend.navigate(Backend.page, v);
        if (Backend.page === 'Strategy') Backend.remember();
    }
    // Categories & goals: the categories as a filter list, selected as the pane selected them.
    readonly property var categories: Backend.page === 'Goals'
        ? Backend.records.filter(r => r.kind === 'category' && (Backend.showArchived ? r.archived : !r.archived)).sort((a, b) => a.order - b.order) : []
    readonly property var chosenCategories: Presentation.categorySelection(Backend.prefs.goalsCategories, Backend.prefs.goalsCategory)
    // Category reorder by dragging the row face, as the habit rows do: the row
    // under the pointer and which half of it say where the dragged one lands.
    property var dropTarget: null
    property bool dropAfter: false
    function categoryAt(sceneY) {
        for (var i = 0; i < catRepeater.count; i++) {
            var it = catRepeater.itemAt(i);
            var top = it.mapToItem(null, 0, 0).y;
            if (sceneY < top + it.height || i === catRepeater.count - 1)
                return { item: it, after: sceneY > top + it.height / 2 };
        }
        return null;
    }

    ColumnLayout {
        anchors.fill: parent; anchors.topMargin: panel.showEntries ? 12 : 0; anchors.bottomMargin: 8; spacing: 2
        Repeater {
            model: panel.showEntries ? panel.entries : []
            delegate: AbstractButton {
                id: entry
                required property var modelData
                objectName: 'side-panel-' + modelData.label
                Layout.fillWidth: true
                implicitHeight: 32
                hoverEnabled: true
                focusPolicy: Qt.StrongFocus
                text: modelData.label
                onClicked: modelData.run()
                contentItem: Text {
                    leftPadding: 16
                    text: entry.text
                    font.family: Theme.font; font.pixelSize: 14
                    font.weight: entry.modelData.active ? Font.Medium : Font.Normal
                    color: entry.modelData.active || entry.hovered ? Theme.text : Theme.muted
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideRight
                }
                background: Rectangle {
                    color: entry.modelData.active ? Theme.raised : entry.hovered ? Theme.hover : 'transparent'
                    Rectangle { anchors.fill: parent; color: 'transparent'; border.width: 2; border.color: Theme.accent; visible: entry.visualFocus }
                }
                Accessible.role: Accessible.PageTab
                Accessible.name: text
                Accessible.selected: modelData.active
            }
        }
        // The page's one filter list, set apart by space alone: the left panel's
        // only partition is the rail's, beneath its first three entries.
        RowLayout {
            Layout.fillWidth: true; Layout.topMargin: 20; Layout.leftMargin: 16; Layout.rightMargin: 8; Layout.bottomMargin: 2
            // In the Goals page's own column the header and sidebar already name the page: no heading.
            visible: Backend.page === 'Goals' && panel.showEntries
            LabelText {
                Layout.fillWidth: true
                text: 'Categories' + (panel.chosenCategories.length ? ' · ' + panel.chosenCategories.length : '')
                font.pixelSize: 12; font.weight: Font.DemiBold; font.letterSpacing: 0.8; font.capitalization: Font.AllUppercase; color: Theme.muted
            }
            SButton { objectName: 'side-panel-new-category'; iconName: 'plus'; accessibleLabel: 'New category'; quiet: true; onClicked: Backend.create('category') }
        }
        LabelText {
            Layout.fillWidth: true; Layout.leftMargin: 16
            visible: Backend.page === 'Goals' && panel.categories.length === 0
            text: 'No categories yet'
            font.pixelSize: 13; color: Theme.muted
        }
        Flickable {
            id: catList
            Layout.fillWidth: true; Layout.fillHeight: true
            visible: Backend.page === 'Goals'
            clip: true; contentWidth: width; contentHeight: catColumn.implicitHeight
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: SScrollBar {}
            Column {
                id: catColumn
                width: catList.width
                // First row of the same list, so it shares the rows' edge and text inset.
                PanelRow {
                    objectName: 'side-panel-All categories'
                    visible: Backend.page === 'Goals' && panel.categories.length > 0
                    width: catColumn.width
                    textInset: 18
                    text: 'All categories'
                    active: panel.chosenCategories.length === 0
                    onClicked: Backend.setting('goalsCategories', [])
                }
                Repeater {
                    id: catRepeater
                    model: panel.categories
                    delegate: PanelRow {
                        id: cat
                        required property var modelData
                        objectName: 'side-panel-category-' + modelData.title
                        width: catColumn.width
                        text: modelData.title
                        twoLines: true
                        barColor: modelData.color || Theme.accent
                        active: panel.chosenCategories.indexOf(modelData.id) >= 0
                        marked: !Backend.privateMode && Backend.isMarkedPrivate(modelData.id)
                        onClicked: Backend.setting('goalsCategories', Presentation.toggled(panel.chosenCategories, modelData.id))
                        onDoubleClicked: Backend.select(modelData.id)
                        onMenuRequested: item => Backend.categoryActionsRequested(modelData, item)
                        Accessible.role: Accessible.CheckBox
                        Accessible.checkable: true
                        Accessible.checked: active
                        lit: hovered || face.containsMouse || face.dragging
                        // A life category, not a filter chip: its own colour as a full-height edge and a faint
                        // wash, the title in the page's text colour, and how many goals it holds.
                        readonly property int goalCount: Backend.records.filter(r => r.kind === 'goal' && !r.hidden && r.category_id === modelData.id).length
                        implicitHeight: 32
                        contentItem: RowLayout {
                            spacing: 6
                            Text {
                                Layout.fillWidth: true
                                leftPadding: 18
                                text: cat.text
                                font.family: Theme.font; font.pixelSize: 15
                                font.weight: cat.active ? Font.DemiBold : Font.Medium
                                color: Theme.text
                                opacity: cat.active || cat.lit ? 1 : 0.88
                                verticalAlignment: Text.AlignVCenter
                                elide: Text.ElideRight
                            }
                            PrivateMarker { visible: cat.marked }
                            Text {
                                visible: !catMenu.visibleNow && cat.goalCount > 0
                                text: cat.goalCount
                                font.family: Theme.font; font.pixelSize: Theme.fontSizeSmall
                                color: Theme.muted
                                verticalAlignment: Text.AlignVCenter
                                rightPadding: 8
                            }
                            SButton {
                                id: catMenu
                                readonly property bool visibleNow: cat.lit || hovered || activeFocus || cat.activeFocus
                                visible: visibleNow
                                iconName: 'more'; quiet: true; implicitWidth: 24; implicitHeight: 28
                                accessibleLabel: 'Actions for ' + cat.text
                                onClicked: cat.menuRequested(this)
                            }
                        }
                        background: Rectangle {
                            color: cat.active ? Theme.raised : cat.lit ? Theme.hover : Qt.tint(Theme.panel, Qt.alpha(cat.barColor, 0.06))
                            Rectangle { objectName: 'category-colour'; width: 4; height: parent.height; color: cat.barColor }
                            Rectangle { anchors.fill: parent; color: 'transparent'; border.width: 2; border.color: Theme.accent; visible: cat.visualFocus }
                        }
                        // The face: a click selects, a double click opens, a vertical drag reorders.
                        MouseArea {
                            id: face
                            property real pressY: 0
                            property bool dragging: false
                            property bool moved: false
                            anchors.fill: parent; anchors.rightMargin: 30
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onPressed: mouse => { pressY = mouse.y; dragging = false; moved = false; cat.forceActiveFocus(); }
                            onPositionChanged: mouse => {
                                if (!pressed) return;
                                if (!dragging && Math.abs(mouse.y - pressY) > 6) dragging = moved = true;
                                if (dragging) {
                                    var hit = panel.categoryAt(mapToItem(null, mouse.x, mouse.y).y);
                                    panel.dropTarget = hit && hit.item !== cat ? hit.item : null;
                                    panel.dropAfter = hit ? hit.after : false;
                                }
                            }
                            onReleased: {
                                if (dragging) {
                                    dragging = false;
                                    if (panel.dropTarget) Backend.placeRecord(cat.modelData, panel.dropTarget.modelData, panel.dropAfter);
                                    panel.dropTarget = null;
                                }
                            }
                            onClicked: if (!moved) cat.clicked()
                            onDoubleClicked: if (!moved) cat.doubleClicked()
                            onCanceled: { dragging = false; panel.dropTarget = null; }
                        }
                        Rectangle {
                            width: parent.width; height: 2; color: Theme.accent
                            visible: panel.dropTarget === cat
                            y: panel.dropAfter ? parent.height - 2 : 0
                        }
                        // A goal dropped on a category moves into it, as on the former pane.
                        DropArea {
                            id: catDrop; anchors.fill: parent; keys: ['goal']
                            onDropped: event => { Backend.moveGoalToCategory(event.source.record, cat.modelData.id); event.acceptProposedAction(); }
                            Rectangle { anchors.fill: parent; color: 'transparent'; border.width: 1; border.color: Theme.accent; visible: catDrop.containsDrag }
                        }
                    }
                }
                // Where the heading's + was: a new category at the end of the list.
                PanelRow {
                    objectName: 'side-panel-new-category-row'
                    visible: !panel.showEntries
                    width: catColumn.width
                    text: 'New category'
                    onClicked: Backend.create('category')
                    contentItem: RowLayout {
                        spacing: 8
                        SIcon { Layout.leftMargin: 18; name: 'plus'; size: 14; color: Theme.muted }
                        Text {
                            Layout.fillWidth: true
                            text: 'New category'
                            font.family: Theme.font; font.pixelSize: 14
                            color: parent.parent.hovered ? Theme.text : Theme.muted
                            verticalAlignment: Text.AlignVCenter
                        }
                    }
                }
            }
        }
        Item { Layout.fillHeight: true; visible: Backend.page !== 'Goals' }
    }
}
