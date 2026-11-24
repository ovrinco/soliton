pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The tracker as it stands in its store: the tabs gathered into a few categories, one tab's table in the
// thin grid, the selected cell's whole text above it. While the page is shown the engine watches the
// store, and a change made anywhere repaints the tab in place. Choices and widths are kept in preferences.
Item {
    id: root
    objectName: 'tracker-page'
    property bool active: false
    property var categories: []
    property string category: ''
    property string tab: ''
    property var page: ({sections: [], rows: [], total: 0})
    property int section: 0
    property var convention: ({})
    property bool reading: false
    property int readSerial: 0
    property int seenRevision: -1
    // Timings for the checks: when a tab was asked for and first painted, when a change arrived and was painted.
    property var paintTiming: ({})
    property var changeTiming: ({})
    property var pendingPaint: null
    readonly property var saved: Backend.prefs.sheet || ({})
    readonly property var categoryNames: categories.map(c => c.name)
    readonly property var tabsHere: (categories.find(c => c.name === category) || {tabs: []}).tabs.map(t => t.name)
    readonly property var sections: page.sections || []
    readonly property var table: sections[Math.min(section, sections.length - 1)] || ({table: '', columns: [], headers: []})
    readonly property string viewKey: tab + '/' + table.table

    function remember(patch) {
        Backend.setting('sheet', Object.assign({}, root.saved, patch));
    }
    function conventionMap(c) {
        var map = {};
        ((c && c.entries) || []).forEach(e => map[e.status] = e);
        return map;
    }
    function loadTabs() {
        Backend.send('tracker-tabs', {}, (r, ok) => {
            if (!ok) return;
            root.categories = r.categories || [];
            var cat = root.categoryNames.indexOf(root.saved.category) >= 0 ? root.saved.category : (root.categoryNames[0] || '');
            root.category = cat;
            var tabs = root.tabsHere;
            root.choose(tabs.indexOf(root.saved.tab) >= 0 ? root.saved.tab : (tabs[0] || ''), true);
        }, true);
    }
    function chooseCategory(name) {
        if (name === root.category) return;
        root.category = name;
        root.choose(root.tabsHere[0] || '', false);
        root.remember({category: name, tab: root.tab});
    }
    function choose(name, restoring) {
        if (!restoring) root.remember({category: root.category, tab: name});
        root.tab = name;
        root.section = 0;
        root.read(false);
    }
    // The whole tab in two reads: a first page painted at once, the rest joined behind it.
    function read(keepPlace) {
        if (!root.tab) return;
        var serial = ++root.readSerial, tab = root.tab, asked = Date.now();
        root.reading = true;
        var first = keepPlace ? 2000 : 200;
        Backend.send('tracker-rows', {tab: tab, limit: first}, (r, ok) => {
            if (serial !== root.readSerial) return;
            if (!ok) { root.reading = false; return; }
            root.convention = root.conventionMap(r.convention);
            root.install(r, r.rows, keepPlace, asked);
            if (r.rows.length < r.total) root.readRest(serial, tab, r.rows, r.total);
            else root.reading = false;
        }, true);
    }
    function readRest(serial, tab, rows, total) {
        Backend.send('tracker-rows', {tab: tab, offset: rows.length, limit: 2000}, (r, ok) => {
            if (serial !== root.readSerial) return;
            if (!ok) { root.reading = false; return; }
            var all = rows.concat(r.rows);
            root.install(r, all, true, 0);
            if (r.rows.length && all.length < total) root.readRest(serial, tab, all, total);
            else root.reading = false;
        }, true);
    }
    function install(reply, rows, keepPlace, asked) {
        root.page = {sections: reply.sections, rows: rows, total: reply.total};
        root.fill(keepPlace);
        if (asked) root.pendingPaint = {tab: root.tab, rows: grid.shown.length, asked: asked, replied: Date.now()};
    }
    // The grid gets the chosen table's rows, its saved widths, order and filter.
    function fill(keepPlace) {
        var t = root.table, rows = (root.page.rows || []).filter(r => r._table === t.table);
        var view = (root.saved.views || {})[root.viewKey] || {};
        grid.columns = t.columns;
        grid.headers = t.headers;
        grid.convention = root.convention;
        grid.widthMap = (root.saved.widths || {})[root.viewKey] || {};
        grid.rows = rows;
        grid.filterColumn = t.columns.indexOf(view.filterColumn) >= 0 ? view.filterColumn : '';
        grid.filterText = view.filterText || '';
        grid.orderColumn = t.columns.indexOf(view.orderColumn) >= 0 ? view.orderColumn : '';
        grid.descending = !!view.descending;
        grid.apply(keepPlace);
    }
    function rememberView(patch) {
        var views = Object.assign({}, root.saved.views || {});
        views[root.viewKey] = Object.assign({}, views[root.viewKey] || {}, patch);
        root.remember({views: views});
    }
    function watch(on) {
        if (on === root.watching) return;
        root.watching = on;
        Backend.watchTracker(on);
    }
    property bool watching: false

    onActiveChanged: {
        root.watch(active);
        if (active) { if (!root.categories.length) root.loadTabs(); else root.read(true); }
    }
    Component.onCompleted: if (active) { root.watch(true); root.loadTabs(); }
    Component.onDestruction: root.watch(false)
    onSectionChanged: if (root.page.rows) root.fill(false)

    // A change in the store while shown: the tab is read again in place.
    Connections {
        target: Backend
        function onTrackerRevisionChanged() {
            var rev = Backend.trackerRevision;
            if (!root.active || rev === null || rev === undefined) return;
            if (root.seenRevision >= 0 && rev !== root.seenRevision && root.tab) {
                root.changeTiming = {revision: rev, seen: Date.now()};
                root.pendingPaint = {tab: root.tab, change: true, asked: Date.now()};
                root.read(true);
            }
            root.seenRevision = rev;
        }
    }
    Connections {
        target: root.Window.window
        function onFrameSwapped() {
            var p = root.pendingPaint;
            if (!p || root.reading && !p.change && grid.shown.length === 0) return;
            root.pendingPaint = null;
            var now = Date.now();
            if (p.change) root.changeTiming = Object.assign({}, root.changeTiming, {painted: now, ms: now - root.changeTiming.seen});
            else root.paintTiming = {tab: p.tab, rows: p.rows, asked: p.asked, replied: p.replied, painted: now, ms: now - p.asked};
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: Theme.space8

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.space8
            Choice {
                objectName: 'sheet-category'
                Layout.preferredWidth: 170
                model: root.categoryNames
                currentIndex: root.categoryNames.indexOf(root.category)
                onActivated: index => root.chooseCategory(root.categoryNames[index])
                Accessible.name: 'Category'
            }
            Choice {
                objectName: 'sheet-tab'
                Layout.preferredWidth: 190
                model: root.tabsHere
                currentIndex: root.tabsHere.indexOf(root.tab)
                onActivated: index => root.choose(root.tabsHere[index], false)
                Accessible.name: 'Tab'
            }
            Repeater {
                model: root.sections.length > 1 ? root.sections : []
                STabButton {
                    required property int index
                    text: String(index + 1)
                    active: root.section === index
                    onClicked: root.section = index
                }
            }
            Item { Layout.fillWidth: true }
            Choice {
                id: filterColumn
                objectName: 'sheet-filter-column'
                Layout.preferredWidth: 150
                readonly property var names: [''].concat(root.table.columns)
                model: ['Any column'].concat(root.table.headers)
                currentIndex: Math.max(0, names.indexOf(grid.filterColumn))
                onActivated: index => { grid.filterColumn = names[index]; root.rememberView({filterColumn: names[index]}); }
                Accessible.name: 'Filter column'
            }
            SSearchField {
                objectName: 'sheet-filter'
                Layout.preferredWidth: 240
                scopeName: 'rows'
                text: grid.filterText
                onTextEdited: { grid.filterText = text; root.rememberView({filterText: text}); }
            }
            LabelText {
                objectName: 'sheet-count'
                color: Theme.muted
                font.pixelSize: Theme.fontSizeSmall
                text: grid.shown.length === grid.rows.length ? String(grid.rows.length) : grid.shown.length + ' / ' + grid.rows.length
            }
        }

        // The content bar: the selected cell's whole text, under its heading.
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(96, Math.max(Theme.controlHeight, cellText.implicitHeight + Theme.space12))
            color: Theme.panel
            border { width: 1; color: Theme.line }
            RowLayout {
                anchors { fill: parent; leftMargin: Theme.space12; rightMargin: Theme.space12 }
                spacing: Theme.space12
                LabelText {
                    objectName: 'sheet-cell-heading'
                    Layout.alignment: Qt.AlignTop
                    Layout.topMargin: Theme.space8
                    text: grid.currentRow ? grid.currentHeader : ''
                    color: Theme.muted
                    font.pixelSize: Theme.fontSizeSmall
                    font.weight: Font.DemiBold
                }
                ScrollView {
                    Layout.fillWidth: true; Layout.fillHeight: true
                    clip: true
                    TextEdit {
                        id: cellText
                        objectName: 'sheet-cell-text'
                        width: parent ? parent.width : 0
                        topPadding: Theme.space8; bottomPadding: Theme.space4
                        readOnly: true
                        selectByMouse: true
                        wrapMode: TextEdit.Wrap
                        text: grid.currentText
                        color: Theme.text
                        selectionColor: Theme.accent
                        selectedTextColor: Theme.accentForeground
                        font.family: Theme.font
                        font.pixelSize: Theme.fontSizeNormal
                    }
                }
            }
        }

        SheetGrid {
            id: grid
            Layout.fillWidth: true
            Layout.fillHeight: true
            onWidthsEdited: map => {
                var widths = Object.assign({}, root.saved.widths || {});
                widths[root.viewKey] = map;
                root.remember({widths: widths});
            }
            onOrderRequested: (column, descending) => {
                grid.orderColumn = column;
                grid.descending = descending;
                root.rememberView({orderColumn: column, descending: descending});
            }
            onTaskRequested: row => Backend.taskFromRow(row)
        }
    }
}
