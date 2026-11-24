pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The thin grid over one table of the tracker, after Zavent Sheets' grid: rows virtualised, columns in
// the store's order with the first frozen, a sticky header ordering by a click and reversing on the
// second, widths dragged at a heading's edge, one selected row and cell. A row with a status takes the
// convention's colour across its whole width; the convention arrives as data with each reply.
Item {
    id: grid
    objectName: 'sheet-grid'

    property var columns: []
    property var headers: []
    property var rows: []
    // Status word to {fill, text, strike}, as the convention names them.
    property var convention: ({})
    property string filterColumn: ''
    property string filterText: ''
    property string orderColumn: ''
    property bool descending: false
    // Column name to width; a column not named starts at its kind's width.
    property var widthMap: ({})
    // Indexes into rows, in the order shown, after the filter.
    property var shown: []
    property int column: 0
    property alias currentIndex: list.currentIndex
    readonly property int rowHeight: Theme.denseRowHeight + 6
    readonly property int headHeight: 30
    readonly property int statusColumn: columns.indexOf('status')
    readonly property var widths: columns.map(c => widthMap[c] || startWidth(c))
    readonly property var currentRow: list.currentIndex >= 0 && list.currentIndex < shown.length ? rows[shown[list.currentIndex]] : null
    readonly property string currentHeader: headers[column] || columns[column] || ''
    readonly property string currentText: currentRow ? text(currentRow[columns[column]]) : ''
    signal widthsEdited(var map)
    signal orderRequested(string column, bool descending)
    signal taskRequested(var row)

    function startWidth(name) {
        if (['purpose', 'intent', 'dod', 'notes', 'outcome', 'summary', 'result', 'fault', 'effect', 'fix'].indexOf(name) >= 0) return 340;
        if (name === 'status' || name === 'standing') return 110;
        if (name === 'id') return 112;
        if (['position', 'age', 'line'].indexOf(name) >= 0) return 70;
        return 140;
    }
    function text(v) {
        if (v === null || v === undefined) return '';
        return typeof v === 'object' ? JSON.stringify(v) : String(v);
    }
    function totalWidth() {
        var w = 0;
        for (var i = 0; i < widths.length; i++) w += widths[i];
        return w;
    }
    function leftOf(col) {
        var w = 0;
        for (var i = 0; i < col; i++) w += widths[i];
        return w;
    }
    // The convention's fill words resolved to the theme's colours, with the ink that reads on each.
    readonly property var tones: {
        var y = Theme.colors.yellow || '#d9a93c', base = Theme.base;
        var fills = {
            'green': Theme.mixed(base, Theme.green, .62), 'green, lighter': Theme.mixed(base, Theme.green, .30),
            'yellow': Theme.mixed(base, y, .62), 'yellow, lighter': Theme.mixed(base, y, .30),
            'grey': Theme.mixed(base, Theme.muted, .45), 'red': Theme.mixed(base, Theme.danger, .62),
            'red, muted': Theme.mixed(base, Theme.mixed(Theme.danger, Theme.muted, .5), .35), 'base': base
        };
        var out = {};
        Object.keys(fills).forEach(k => out[k] = {fill: fills[k], ink: Theme.readable(Theme.text, fills[k]), muted: Theme.readable(Theme.muted, fills[k])});
        return out;
    }
    function tone(status) {
        var rule = convention[status];
        return rule ? (tones[rule.fill] || tones.base) : tones.base;
    }
    function apply(keepPlace) {
        var keep = keepPlace && currentRow ? text(currentRow[columns[0]]) : null;
        var needle = filterText.toLowerCase(), col = filterColumn;
        var out = [];
        for (var i = 0; i < rows.length; i++) {
            if (needle.length) {
                var hay = col ? text(rows[i][col]) : columns.map(c => text(rows[i][c])).join(' ');
                if (hay.toLowerCase().indexOf(needle) < 0) continue;
            }
            out.push(i);
        }
        if (orderColumn && columns.indexOf(orderColumn) >= 0) {
            var k = orderColumn, sign = descending ? -1 : 1;
            var key = v => typeof v === 'number' ? v : text(v).toLowerCase();
            out.sort((a, b) => {
                var x = key(rows[a][k]), y = key(rows[b][k]);
                return x < y ? -sign : x > y ? sign : a - b;
            });
        }
        var at = -1;
        if (keep !== null) for (var j = 0; j < out.length; j++) if (text(rows[out[j]][columns[0]]) === keep) { at = j; break; }
        var y = list.contentY;
        shown = out;
        list.currentIndex = at >= 0 ? at : keepPlace ? Math.min(list.currentIndex, out.length - 1) : (out.length ? 0 : -1);
        if (keepPlace) list.contentY = y;
        else list.positionViewAtBeginning();
    }
    // A heading click orders by its column, a second reverses it, a third returns to the store's own order.
    function order(name) {
        if (orderColumn === name && descending) orderRequested('', false);
        else orderRequested(name, orderColumn === name);
    }
    function showColumn(col) {
        var left = leftOf(col), frozen = widths[0] || 0, right = left + (widths[col] || 0);
        if (col === 0) return;
        if (left - frozen < list.contentX) list.contentX = Math.max(0, left - frozen);
        else if (right > list.contentX + list.width) list.contentX = Math.min(left - frozen, right - list.width);
    }
    function move(by) {
        if (!shown.length) return;
        list.currentIndex = Math.max(0, Math.min(shown.length - 1, list.currentIndex + by));
        list.positionViewAtIndex(list.currentIndex, ListView.Contain);
    }
    // What the delegates paint, read back from the tree for checks.
    function painted() {
        var out = [], items = list.contentItem.children;
        for (var i = 0; i < items.length; i++) {
            var d = items[i];
            if (d.rowIndex === undefined || !d.visible || d.rowIndex < 0 || d.y + d.height <= list.contentY || d.y >= list.contentY + list.height) continue;
            out.push({index: d.index, id: text(d.cells[columns[0]]), status: d.statusWord, fill: String(d.tint)});
        }
        out.sort((a, b) => a.index - b.index);
        return out;
    }

    onFilterTextChanged: apply(false)
    onFilterColumnChanged: apply(false)
    onOrderColumnChanged: apply(true)
    onDescendingChanged: apply(true)

    Rectangle { anchors.fill: parent; color: Theme.base }

    ListView {
        id: list
        objectName: 'sheet-rows'
        anchors.fill: parent
        clip: true
        focus: true
        activeFocusOnTab: true
        model: grid.shown.length
        reuseItems: true
        cacheBuffer: 400
        contentWidth: Math.max(width, grid.totalWidth())
        flickableDirection: Flickable.AutoFlickIfNeeded
        boundsBehavior: Flickable.StopAtBounds
        highlightFollowsCurrentItem: false
        headerPositioning: ListView.OverlayHeader
        ScrollBar.vertical: SScrollBar {}
        ScrollBar.horizontal: SScrollBar {}

        Keys.onPressed: event => {
            var page = Math.max(1, Math.floor(list.height / grid.rowHeight) - 1);
            switch (event.key) {
            case Qt.Key_Up: grid.move(-1); break;
            case Qt.Key_Down: grid.move(1); break;
            case Qt.Key_PageUp: grid.move(-page); break;
            case Qt.Key_PageDown: grid.move(page); break;
            case Qt.Key_Home: grid.move(-grid.shown.length); break;
            case Qt.Key_End: grid.move(grid.shown.length); break;
            case Qt.Key_Left: grid.column = Math.max(0, grid.column - 1); grid.showColumn(grid.column); break;
            case Qt.Key_Right: grid.column = Math.min(grid.columns.length - 1, grid.column + 1); grid.showColumn(grid.column); break;
            default: return;
            }
            event.accepted = true;
        }

        // The sticky header: a heading per column, the first held in place; a heading's right edge resizes it.
        header: Rectangle {
            id: head
            z: 3
            width: list.contentWidth
            height: grid.headHeight
            color: Theme.panel
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line }
            Repeater {
                model: grid.columns
                Rectangle {
                    id: heading
                    required property string modelData
                    required property int index
                    objectName: 'sheet-heading-' + modelData
                    x: index === 0 ? list.contentX : grid.leftOf(index)
                    z: index === 0 ? 2 : 1
                    width: grid.widths[index] || 120
                    height: grid.headHeight
                    color: Theme.panel
                    readonly property bool ordered: grid.orderColumn === heading.modelData
                    LabelText {
                        id: headingWord
                        anchors { left: parent.left; right: parent.right; top: parent.top; bottom: parent.bottom; leftMargin: Theme.space8; rightMargin: Theme.space8 + (heading.ordered ? orderMark.width + Theme.space4 : 0) }
                        verticalAlignment: Text.AlignVCenter
                        elide: Text.ElideRight
                        text: grid.headers[heading.index] || heading.modelData
                        color: Theme.muted
                        font.pixelSize: Theme.fontSizeSmall
                        font.weight: Font.DemiBold
                    }
                    // The order's direction: the chevron points down while descending, up while ascending.
                    SIcon {
                        id: orderMark
                        objectName: 'sheet-order-mark'
                        visible: heading.ordered
                        x: Math.min(headingWord.x + headingWord.contentWidth + Theme.space4, heading.width - width - Theme.space8)
                        anchors.verticalCenter: parent.verticalCenter
                        size: 12
                        name: 'down'
                        rotation: grid.descending ? 0 : 180
                        color: Theme.muted
                    }
                    Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: Theme.line }
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: grid.order(heading.modelData)
                    }
                    MouseArea {
                        anchors { right: parent.right; top: parent.top; bottom: parent.bottom }
                        width: 6
                        cursorShape: Qt.SplitHCursor
                        property real startX: 0
                        property real startWidth: 0
                        onPressed: mouse => { startX = mapToItem(null, mouse.x, 0).x; startWidth = heading.width; }
                        onPositionChanged: mouse => {
                            var m = Object.assign({}, grid.widthMap);
                            m[heading.modelData] = Math.round(Math.max(48, startWidth + mapToItem(null, mouse.x, 0).x - startX));
                            grid.widthMap = m;
                        }
                        onReleased: grid.widthsEdited(grid.widthMap)
                    }
                }
            }
        }

        delegate: Item {
            id: line
            required property int index
            readonly property int rowIndex: grid.shown[index] === undefined ? -1 : grid.shown[index]
            readonly property var cells: rowIndex >= 0 ? grid.rows[rowIndex] : ({})
            readonly property string statusWord: grid.statusColumn >= 0 ? grid.text(cells.status) : ''
            readonly property var rule: grid.convention[statusWord] || null
            readonly property var toneOf: grid.tone(statusWord)
            readonly property color tint: toneOf.fill
            readonly property bool struck: !!rule && rule.strike === true
            readonly property bool current: ListView.isCurrentItem
            width: list.contentWidth
            height: grid.rowHeight

            Rectangle { anchors.fill: parent; color: line.tint }
            Rectangle {
                anchors.fill: parent
                color: line.current ? Qt.alpha(Theme.selection, .55) : rowMouse.containsMouse ? Qt.alpha(Theme.hover, .5) : 'transparent'
            }
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line }
            MouseArea {
                id: rowMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: mouse => {
                    list.currentIndex = line.index;
                    grid.column = mouse.x < list.contentX + (grid.widths[0] || 0) ? 0 : cellAt(mouse.x);
                    list.forceActiveFocus();
                }
                function cellAt(x) {
                    var left = 0;
                    for (var i = 0; i < grid.widths.length; i++) { left += grid.widths[i]; if (x < left) return i; }
                    return grid.widths.length - 1;
                }
            }
            Repeater {
                model: grid.columns.length
                Rectangle {
                    id: cell
                    required property int index
                    readonly property string value: grid.text(line.cells[grid.columns[index]])
                    readonly property bool selected: line.current && grid.column === index
                    x: index === 0 ? list.contentX : grid.leftOf(index)
                    z: index === 0 ? 2 : 1
                    width: grid.widths[index] || 120
                    height: grid.rowHeight
                    // The frozen column paints the row's colour so the cells beneath never show through.
                    color: index === 0 ? line.tint : 'transparent'
                    Rectangle {
                        anchors.fill: parent
                        visible: cell.index === 0 && (line.current || rowMouse.containsMouse)
                        color: line.current ? Qt.alpha(Theme.selection, .55) : Qt.alpha(Theme.hover, .5)
                    }
                    Rectangle {
                        anchors.fill: parent
                        visible: cell.selected
                        color: 'transparent'
                        border { width: 1; color: Theme.accent }
                    }
                    Text {
                        anchors { fill: parent; leftMargin: Theme.space8; rightMargin: Theme.space8 }
                        verticalAlignment: Text.AlignVCenter
                        wrapMode: Text.Wrap
                        maximumLineCount: 2
                        elide: Text.ElideRight
                        text: cell.value
                        font.family: Theme.font
                        font.pixelSize: Theme.fontSizeSmall
                        font.strikeout: line.struck
                        font.weight: cell.index === grid.statusColumn ? Font.DemiBold : Font.Normal
                        color: line.rule && line.rule.text === 'muted text' ? line.toneOf.muted : line.toneOf.ink
                    }
                    Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: Theme.line; visible: cell.index === 0 }
                }
            }
            // The row's three-dot menu, at the visible right edge, on hover or selection, on its own backing.
            Rectangle {
                x: moreButton.x - Theme.space4; y: moreButton.y - Theme.space4 / 2
                z: 3
                width: moreButton.width + Theme.space8; height: moreButton.height + Theme.space4
                radius: Theme.cornerRadius
                visible: moreButton.visible
                color: Theme.raised
                border { width: 1; color: Theme.line }
            }
            SButton {
                id: moreButton
                objectName: 'sheet-row-more'
                x: list.contentX + list.width - width - Theme.space24
                anchors.verticalCenter: parent.verticalCenter
                z: 4
                width: Theme.innerButtonSize; height: Theme.innerButtonSize
                implicitWidth: Theme.innerButtonSize; implicitHeight: Theme.innerButtonSize
                iconName: 'more'; accessibleLabel: 'Row actions'; quiet: true
                focusPolicy: Qt.NoFocus
                visible: line.current || rowMouse.containsMouse || hovered || rowMenu.visible
                onClicked: { list.currentIndex = line.index; rowMenu.open(); }
            }
            SPopup {
                id: rowMenu
                trigger: moreButton
                alignRight: true
                parent: Overlay.overlay
                width: 180
                padding: 6
                contentItem: ColumnLayout {
                    spacing: 2
                    MenuAction { objectName: 'sheet-row-task'; text: 'Task'; iconName: 'plus'; onClicked: { rowMenu.close(); grid.taskRequested(line.cells); } }
                }
            }
        }
    }
}
