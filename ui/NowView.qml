pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io

// Hearth, Now: the day's few items as the engine ranks and bounds them, each
// with its reason and what it stands on, then one count of what was left out.
// The bound, the order and the two windows are his settings, kept in the
// view's own menu. Nothing is ranked, counted or cut here.
FocusScope {
    id: view
    objectName: 'now-view'

    readonly property var items: Backend.nowList.items || []
    readonly property int leftOut: Backend.nowList.left_out || 0
    property string focusId: ''
    property var drag: null

    // The engine's reason codes as words, in one place.
    readonly property var whyWords: ({'due-today': 'Due today', 'scheduled-now': 'Today', 'bundle-open': 'Window open', 'goes-off': 'Goes off'})
    // A source that is the item itself is named by the page that holds it.
    readonly property var homeWords: ({task: 'Tasks', habit: 'Habits', lot: 'Kitchen'})
    readonly property var kindWords: ({task: 'Tasks', habit: 'Habits', lot: 'Kitchen'})
    readonly property var order: {
        var o = (Backend.prefs.nowOrder || []).filter(k => ['task', 'habit', 'lot'].indexOf(k) >= 0);
        ['task', 'habit', 'lot'].forEach(k => { if (o.indexOf(k) < 0) o.push(k); });
        return o;
    }
    readonly property int bound: [5, 7, 10, 15].indexOf(Backend.prefs.nowBound) >= 0 ? Backend.prefs.nowBound : 7

    function headingDay() {
        var day = Backend.nowList.day || Backend.today, p = day.split('-');
        var text = new Date(Number(p[0]), Number(p[1]) - 1, Number(p[2])).toLocaleDateString(Qt.locale('en_GB'), 'dddd d MMMM');
        return day === Backend.today ? 'Today, ' + text : text;
    }
    function whyColor(item) {
        if (item.kind === 'task') return Backend.color(item.id);
        if (item.kind === 'lot') return Theme.dangerText;
        return Theme.accentText;
    }
    function sourceWords(item) {
        return (item.sources || []).map(s => s.kind === item.kind && s.id === item.id ? homeWords[s.kind] : s.title).join(' · ');
    }
    function when(item) {
        if (item.at) return item.at;
        if (!item.on) return '';
        var p = item.on.split('-');
        return new Date(Number(p[0]), Number(p[1]) - 1, Number(p[2])).toLocaleDateString(Qt.locale('en_GB'), 'ddd d MMM');
    }
    function bundleOf(item) {
        var b = (item.sources || []).find(s => s.kind === 'bundle');
        return b ? b.id : '';
    }
    // Each item opens where it lives.
    function open(item) {
        if (item.kind === 'task') {
            Backend.navigate('Tasks', 'Tasks');
            Backend.select(item.id);
        } else if (item.kind === 'habit') {
            Backend.habitDay = '';
            Backend.navigate('Hearth', 'Habits');
            Backend.habitFocusRequest = {serial: Backend.habitFocusRequest.serial + 1, bundle: bundleOf(item)};
        } else if (item.kind === 'lot') {
            Backend.openNowLot(item.id);
        }
    }
    function done(item) {
        var r = Backend.get(item.id);
        if (r) Backend.complete(r, () => Backend.loadNow());
    }
    function checkIn(item) {
        var h = Backend.habit(item.id);
        if (!h) return;
        Backend.habitDay = '';
        Backend.habitCheckIn(item.id, h.direction === 'break' ? Math.round(Backend.habitValue(item.id) || 0) + 1 : 1);
    }
    function actionsFor(item) {
        var a = [{text: 'Open', run: () => view.open(item)}];
        if (item.kind === 'task') a.push({text: 'Done', run: () => view.done(item)});
        if (item.kind === 'habit') a.push({text: 'Check in', run: () => view.checkIn(item)});
        return a;
    }
    function moveKind(kind, index) {
        var o = order.filter(k => k !== kind);
        o.splice(Math.max(0, Math.min(o.length, index)), 0, kind);
        if (JSON.stringify(o) !== JSON.stringify(order)) Backend.nowSetting('nowOrder', o);
    }

    Component.onCompleted: Backend.loadNow()
    Connections {
        target: Backend
        // A check-in made anywhere changes what is still open.
        function onHabitListChanged() { if (Backend.view === 'Now') Backend.loadNow(); }
    }

    Keys.onPressed: event => {
        var i = view.items.findIndex(x => x.id === view.focusId);
        if ((event.modifiers & Qt.AltModifier) && event.key === Qt.Key_R) { Backend.loadNow(); event.accepted = true; }
        else if (event.key === Qt.Key_Down || event.key === Qt.Key_Up) {
            if (view.items.length) {
                var n = i < 0 ? 0 : Math.max(0, Math.min(view.items.length - 1, i + (event.key === Qt.Key_Down ? 1 : -1)));
                view.focusId = view.items[n].id;
            }
            event.accepted = true;
        }
        else if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter) && i >= 0) { view.open(view.items[i]); event.accepted = true; }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            spacing: 4
            Item { Layout.fillWidth: true }
            LabelText {
                objectName: 'now-day'
                text: view.headingDay()
                font.pixelSize: 14; font.weight: Font.Medium
                Layout.rightMargin: 8
            }
            SButton {
                id: settingsButton
                objectName: 'now-more'
                iconName: 'more'; accessibleLabel: 'Now settings'; quiet: true
                onClicked: settingsMenu.open()
            }
        }

        Column {
            id: list
            objectName: 'now-list'
            Layout.fillWidth: true
            Repeater {
                model: view.items
                delegate: Item {
                    id: row
                    required property var modelData
                    required property int index
                    readonly property bool focused: view.activeFocus && view.focusId === modelData.id
                    objectName: 'now-row-' + index
                    width: list.width
                    height: Theme.denseRowHeight
                    MouseArea {
                        id: face
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: view.open(row.modelData)
                    }
                    Rectangle { anchors.fill: parent; color: Theme.hover; visible: face.containsMouse || rowMore.hovered || rowMenu.visible }
                    Rectangle { anchors.fill: parent; color: 'transparent'; border.width: 2; border.color: Theme.accent; visible: row.focused }
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8; anchors.rightMargin: 4
                        spacing: 16
                        LabelText {
                            Layout.preferredWidth: Math.min(360, list.width * .4)
                            text: row.modelData.title
                            font.pixelSize: 14
                            elide: Text.ElideRight
                        }
                        LabelText {
                            objectName: 'now-why-' + row.index
                            Layout.preferredWidth: 110
                            text: view.whyWords[row.modelData.why] || ''
                            color: view.whyColor(row.modelData)
                            font.pixelSize: 13; font.weight: Font.Medium
                        }
                        LabelText {
                            Layout.fillWidth: true
                            text: '· ' + view.sourceWords(row.modelData)
                            color: Theme.muted
                            font.pixelSize: 13
                            elide: Text.ElideRight
                        }
                        LabelText {
                            text: view.when(row.modelData)
                            visible: text !== ''
                            color: Theme.muted
                            font.pixelSize: 13
                        }
                        Item {
                            implicitWidth: 28; implicitHeight: 28
                            SButton {
                                id: rowMore
                                anchors.fill: parent
                                objectName: 'now-row-more-' + row.index
                                iconName: 'more'; accessibleLabel: 'More actions'; quiet: true
                                focusPolicy: Qt.NoFocus
                                opacity: face.containsMouse || hovered || rowMenu.visible || row.focused ? 1 : 0
                                onClicked: rowMenu.open()
                            }
                        }
                    }
                    SPopup {
                        id: rowMenu
                        trigger: rowMore
                        alignRight: true
                        parent: Overlay.overlay
                        width: 180
                        padding: 6
                        contentItem: ColumnLayout {
                            spacing: 2
                            Repeater {
                                model: view.actionsFor(row.modelData)
                                delegate: MenuAction {
                                    required property var modelData
                                    objectName: 'now-row-action-' + row.index + '-' + modelData.text
                                    text: modelData.text
                                    onClicked: { rowMenu.close(); modelData.run(); }
                                }
                            }
                        }
                    }
                }
            }
        }

        LabelText {
            objectName: 'now-left-out'
            visible: view.leftOut > 0
            text: view.leftOut + ' more'
            color: Theme.muted
            Layout.leftMargin: 8
        }
        Item { Layout.fillHeight: true }
    }

    // His objective: the bound, the order of the kinds and the two windows.
    SPopup {
        id: settingsMenu
        objectName: 'now-settings'
        trigger: settingsButton
        alignRight: true
        parent: Overlay.overlay
        width: 260
        padding: 12
        contentItem: GridLayout {
            columns: 2; columnSpacing: 12; rowSpacing: 8
            LabelText { text: 'Bound'; color: Theme.muted; font.pixelSize: 13 }
            Choice {
                id: boundChoice
                objectName: 'now-bound'
                Layout.fillWidth: true
                model: [5, 7, 10, 15]
                currentIndex: Math.max(0, model.indexOf(view.bound))
                onActivated: index => Backend.nowSetting('nowBound', boundChoice.model[index])
                Accessible.name: 'Bound'
            }
            LabelText { text: 'Order'; color: Theme.muted; font.pixelSize: 13; Layout.alignment: Qt.AlignTop; Layout.topMargin: 6 }
            Column {
                id: kinds
                objectName: 'now-order'
                Layout.fillWidth: true
                Repeater {
                    model: view.order
                    delegate: Item {
                        id: kindRow
                        required property string modelData
                        required property int index
                        objectName: 'now-order-' + modelData
                        width: kinds.width
                        height: Theme.denseRowHeight
                        Rectangle {
                            width: parent.width; height: 2; y: -1
                            color: Theme.accent
                            visible: !!view.drag && view.drag.index === kindRow.index && view.drag.kind !== kindRow.modelData
                        }
                        Rectangle {
                            width: parent.width; height: 2; y: parent.height - 1
                            color: Theme.accent
                            visible: !!view.drag && kindRow.index === view.order.length - 1 && view.drag.index >= view.order.length
                        }
                        Rectangle { anchors.fill: parent; color: grab.dragging ? Theme.raised : grab.containsMouse ? Theme.hover : 'transparent' }
                        LabelText { anchors.verticalCenter: parent.verticalCenter; x: 8; text: view.kindWords[kindRow.modelData] }
                        MouseArea {
                            id: grab
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: dragging ? Qt.ClosedHandCursor : Qt.OpenHandCursor
                            property real pressY: 0
                            property bool dragging: false
                            function slot(y) { return Math.max(0, Math.min(view.order.length, Math.round((kindRow.y + y) / Theme.denseRowHeight))); }
                            onPressed: mouse => { pressY = mouse.y; dragging = false; }
                            onPositionChanged: mouse => {
                                if (!dragging && Math.abs(mouse.y - pressY) > 6) dragging = true;
                                if (dragging) view.drag = {kind: kindRow.modelData, index: slot(mouse.y - pressY + kindRow.height / 2)};
                            }
                            onReleased: mouse => {
                                if (dragging) {
                                    dragging = false;
                                    var to = slot(mouse.y - pressY + kindRow.height / 2);
                                    view.drag = null;
                                    view.moveKind(kindRow.modelData, to > kindRow.index ? to - 1 : to);
                                }
                            }
                            onCanceled: { dragging = false; view.drag = null; }
                        }
                    }
                }
            }
            LabelText { text: 'Morning until'; color: Theme.muted; font.pixelSize: 13 }
            TimeSetting { objectName: 'now-morning-until'; key: 'nowMorningUntil'; fallback: '12:00'; Accessible.name: 'Morning until' }
            LabelText { text: 'Evening from'; color: Theme.muted; font.pixelSize: 13 }
            TimeSetting { objectName: 'now-evening-from'; key: 'nowEveningFrom'; fallback: '18:00'; Accessible.name: 'Evening from' }
        }
    }

    component TimeSetting: Field {
        id: time
        property string key: ''
        property string fallback: ''
        readonly property string stored: typeof Backend.prefs[key] === 'string' ? Backend.prefs[key] : fallback
        Layout.fillWidth: true
        text: stored
        placeholderText: 'HH:MM'
        onTextEdited: invalid = false
        onEditingFinished: {
            if (text === stored) return;
            if (!/^([01][0-9]|2[0-3]):[0-5][0-9]$/.test(text)) { invalid = true; return; }
            Backend.nowSetting(key, text);
        }
    }

    // The private display's proof reads the rows and names the engine's day
    // and time; it exists only under automation.
    IpcHandler {
        target: 'soliton-now'
        enabled: Quickshell.env('SOLITON_AUTOMATION') === '1'
        function at(day: string, time: string): void {
            Backend.nowAt = {day: day, time: time};
            Backend.loadNow();
        }
        function rows(): string {
            return JSON.stringify({day: view.headingDay(), leftOut: view.leftOut, foot: view.leftOut > 0 ? view.leftOut + ' more' : '',
                rows: view.items.map(x => ({title: x.title, why: view.whyWords[x.why] || '', color: String(view.whyColor(x)), sources: view.sourceWords(x), when: view.when(x)}))});
        }
    }
}
