pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Monitors, arranged live: the outputs the compositor reports, drawn to scale
// and dragged into place, each with its mode, scale and orientation beside
// the arrangement; workspaces as chips under their output, dragged or picked
// to another. A change shows as one line and applies with the engine's timed
// keep or revert. With one output on, the page shows only the survivor, offers
// to record the arrangement for that state and moves stranded workspaces to it.
ColumnLayout {
    id: root
    Layout.minimumWidth: 0
    property bool active: false
    property bool monitors: true
    spacing: Theme.space12

    property var snapshot: ({})
    property var fallback: ({})
    property string snapshotText: ''
    property var draftOutputs: ({})
    property var draftWorkspaces: ({})
    property string selectedName: ''
    property var plan: null
    property string refusal: ''
    property string error: ''
    property bool busy: false
    property string pending: ''
    property int pendingSeconds: 0
    readonly property var orientations: ['Normal', '90°', '180°', '270°', 'Flipped', 'Flipped 90°', 'Flipped 180°', 'Flipped 270°']

    // The outputs on: the engine's active list. A screen asleep stays in
    // place with its workspaces; it is drawn but never arranged.
    readonly property var outputs: snapshot.items || []
    readonly property var outputNames: outputs.map(row => row.name)
    readonly property var asleep: (snapshot.allItems || []).filter(row => !row.disabled && (row.dpmsStatus === false) && outputNames.indexOf(row.name) < 0 && row.width > 0)
    readonly property var asleepNames: asleep.map(row => row.name)
    readonly property var workspaceRows: (snapshot.workspaces || []).filter(row => row.id > 0).sort((a, b) => a.id - b.id)
    readonly property var stranded: workspaceRows.filter(row => outputNames.indexOf(row.monitor) < 0 && asleepNames.indexOf(row.monitor) < 0)
    readonly property int strandedWindows: stranded.reduce((sum, row) => sum + (row.windows || 0), 0)
    readonly property var selected: outputs.find(row => row.name === selectedName) || outputs[0] || null

    function modeOf(row) {
        var prefix = row.width + 'x' + row.height + '@';
        var modes = (row.availableModes || []).filter(mode => mode.indexOf(prefix) === 0);
        var rate = row.refreshRate || 0;
        modes.sort((a, b) => Math.abs(parseFloat(a.split('@')[1]) - rate) - Math.abs(parseFloat(b.split('@')[1]) - rate));
        return modes[0] || prefix + rate.toFixed(2) + 'Hz';
    }
    function readOutput(row) {
        return {mode: modeOf(row), scale: Number(row.scale || 1), x: row.x, y: row.y, transform: row.transform || 0};
    }
    function readAssignments() {
        var placed = {};
        workspaceRows.forEach(row => {
            if (outputNames.indexOf(row.monitor) >= 0) placed[row.name] = row.monitor;
            else if (asleepNames.indexOf(row.monitor) < 0 && outputNames.length) placed[row.name] = outputNames[0];
        });
        return placed;
    }
    function resetDraft() {
        var drafted = {};
        outputs.forEach(row => drafted[row.name] = readOutput(row));
        draftOutputs = drafted;
        draftWorkspaces = readAssignments();
        plan = null; refusal = '';
    }

    // What will change, in plain words: moved, resized, turned, workspaces moved.
    readonly property var changes: {
        var lines = [];
        outputs.forEach(row => {
            var was = readOutput(row), now = draftOutputs[row.name];
            if (!now) return;
            if (now.x !== was.x || now.y !== was.y) lines.push(row.name + ' to ' + now.x + ', ' + now.y);
            if (now.mode !== was.mode) lines.push(row.name + ' ' + now.mode.replace(/\.\d+Hz$/, ' Hz').replace('@', ' at '));
            if (Number(now.scale) !== Number(was.scale)) lines.push(row.name + ' scale ' + now.scale);
            if (now.transform !== was.transform) lines.push(row.name + ' ' + orientations[now.transform].toLowerCase());
        });
        var moved = {};
        workspaceRows.forEach(row => {
            var target = draftWorkspaces[row.name];
            if (target && target !== row.monitor) (moved[target] = moved[target] || []).push(row.name);
        });
        Object.keys(moved).sort().forEach(target => lines.push((moved[target].length > 1 ? 'workspaces ' : 'workspace ') + moved[target].join(', ') + ' to ' + target));
        return lines;
    }
    readonly property bool changed: changes.length > 0

    function request() {
        var placed = {};
        Object.keys(draftWorkspaces).forEach(name => { if (outputNames.indexOf(draftWorkspaces[name]) >= 0) placed[name] = draftWorkspaces[name]; });
        return {outputs: draftOutputs, workspaces: placed, revision: snapshot.revision};
    }

    function load() {
        if (busy) return;
        Backend.send('desktop-monitors', {}, (result, ok, failure) => {
            if (!ok) { error = failure || ''; return; }
            var text = JSON.stringify(result);
            if (text === snapshotText) return;
            var sameOutputs = snapshot.activeKey === result.activeKey;
            snapshotText = text; snapshot = result;
            if (!changed || !sameOutputs) resetDraft();
        }, true);
        Backend.send('desktop-fallback-read', {}, (result, ok) => { if (ok) fallback = result || {}; }, true);
    }
    function updateOutput(name, values) {
        var updated = Object.assign({}, draftOutputs);
        updated[name] = Object.assign({}, updated[name], values);
        draftOutputs = updated;
    }
    function place(workspace, output) {
        var updated = Object.assign({}, draftWorkspaces);
        updated[workspace] = output;
        draftWorkspaces = updated;
    }
    function apply() {
        if (!plan || busy) return;
        busy = true;
        Backend.send('desktop-arrangement-apply', request(), (receipt, ok, failure) => {
            busy = false;
            if (!ok) { error = failure || ''; return; }
            pending = receipt.token || ''; pendingSeconds = receipt.confirm_seconds || 0;
            snapshotText = ''; load();
        }, true);
    }
    function confirm(keep) {
        if (!pending) return;
        var token = pending;
        pending = ''; busy = true;
        Backend.send('desktop-arrangement-confirm', {token: token, keep: keep}, (result, ok, failure) => {
            busy = false; if (!ok) error = failure || '';
            snapshotText = ''; resetDraft(); load();
        }, true);
    }

    // The logical size an output takes: its mode over its scale, turned a quarter swaps.
    function logical(name) {
        var draft = draftOutputs[name] || {};
        var size = String(draft.mode || '').match(/^(\d+)x(\d+)@/);
        var row = outputs.find(item => item.name === name) || asleep.find(item => item.name === name) || {};
        var width = size ? Number(size[1]) : row.width, height = size ? Number(size[2]) : row.height;
        var scale = Number(draft.scale || row.scale || 1), turned = (draft.transform !== undefined ? draft.transform : row.transform || 0) % 2;
        return {w: Math.round((turned ? height : width) / scale), h: Math.round((turned ? width : height) / scale)};
    }
    readonly property var rects: outputs.map(row => {
        var draft = draftOutputs[row.name] || readOutput(row), size = logical(row.name);
        return {name: row.name, x: draft.x, y: draft.y, w: size.w, h: size.h, asleep: false};
    }).concat(asleep.map(row => ({name: row.name, x: row.x, y: row.y, w: logical(row.name).w, h: logical(row.name).h, asleep: true})))
    readonly property real minX: rects.length ? Math.min(...rects.map(rect => rect.x)) : 0
    readonly property real minY: rects.length ? Math.min(...rects.map(rect => rect.y)) : 0
    readonly property real spanX: rects.length ? Math.max(1, Math.max(...rects.map(rect => rect.x + rect.w)) - minX) : 1
    readonly property real spanY: rects.length ? Math.max(1, Math.max(...rects.map(rect => rect.y + rect.h)) - minY) : 1

    // A dropped output snaps beside the nearest other one: the side its
    // centre fell on, edges aligned when close; then the layout starts at 0, 0.
    function drop(name, x, y) {
        var size = logical(name), others = rects.filter(rect => rect.name !== name && !rect.asleep);
        if (!others.length) return;
        var cx = x + size.w / 2, cy = y + size.h / 2;
        var other = others.slice().sort((a, b) => Math.hypot(a.x + a.w / 2 - cx, a.y + a.h / 2 - cy) - Math.hypot(b.x + b.w / 2 - cx, b.y + b.h / 2 - cy))[0];
        var dx = (cx - other.x - other.w / 2) / ((size.w + other.w) / 2), dy = (cy - other.y - other.h / 2) / ((size.h + other.h) / 2);
        var align = (value, low, length, otherLength) => {
            var near = [low, low + otherLength - length, low + (otherLength - length) / 2].sort((a, b) => Math.abs(a - value) - Math.abs(b - value))[0];
            var reach = 0.08 * Math.max(length, otherLength);
            return Math.abs(near - value) <= reach ? near : Math.max(low - length + 1, Math.min(low + otherLength - 1, value));
        };
        if (Math.abs(dx) >= Math.abs(dy)) { x = dx > 0 ? other.x + other.w : other.x - size.w; y = align(y, other.y, size.h, other.h); }
        else { y = dy > 0 ? other.y + other.h : other.y - size.h; x = align(x, other.x, size.w, other.w); }
        var updated = Object.assign({}, draftOutputs);
        updated[name] = Object.assign({}, updated[name], {x: Math.round(x), y: Math.round(y)});
        var names = Object.keys(updated);
        var lowX = Math.min(...names.map(key => updated[key].x)), lowY = Math.min(...names.map(key => updated[key].y));
        names.forEach(key => updated[key] = Object.assign({}, updated[key], {x: updated[key].x - lowX, y: updated[key].y - lowY}));
        draftOutputs = updated;
    }
    function scalesFor(mode, current) {
        var size = String(mode || '').match(/^(\d+)x(\d+)@/);
        var whole = scale => !size || [Number(size[1]), Number(size[2])].every(pixels => Math.abs(pixels / scale - Math.round(pixels / scale)) <= 0.01);
        var list = [0.5, 0.75, 1, 1.25, 1.5, 1.6, 1.75, 2, 2.5, 3, 4].filter(whole);
        if (current && list.indexOf(Number(current)) < 0) list.push(Number(current));
        return list.sort((a, b) => a - b).map(String);
    }

    onActiveChanged: if (active) { snapshotText = ''; load(); }
    Connections {
        target: SystemSettings
        function onChanged(domain) { if (root.active && domain === 'devices') root.load(); }
    }
    // The page follows the compositor: a screen moved or turned off elsewhere shows here.
    Timer { interval: 3000; repeat: true; running: root.active && !root.pending && !root.busy; onTriggered: root.load() }
    Timer {
        id: review
        interval: 200
        onTriggered: {
            if (!root.changed) { root.plan = null; root.refusal = ''; return; }
            var asked = JSON.stringify(root.request());
            Backend.send('desktop-arrangement-preview', root.request(), (result, ok, failure) => {
                if (JSON.stringify(root.request()) !== asked) return;
                root.plan = ok ? result : null; root.refusal = ok ? '' : (failure || '');
            }, true);
        }
    }
    onDraftOutputsChanged: { plan = null; review.restart(); }
    onDraftWorkspacesChanged: { plan = null; review.restart(); }
    Timer {
        interval: 1000; repeat: true; running: !!root.pending
        onTriggered: { root.pendingSeconds--; if (root.pendingSeconds <= 0) root.confirm(false); }
    }
    NoticePopup { scope: root; text: root.error; onDismissed: root.error = '' }

    RowLayout {
        Layout.fillWidth: true; Layout.fillHeight: true; Layout.minimumHeight: 0
        spacing: Theme.space16

        Rectangle {
            id: arrangement
            objectName: 'monitor-arrangement'
            Layout.fillWidth: true; Layout.fillHeight: true; Layout.minimumWidth: 0
            color: Theme.panel; border.color: Theme.line; radius: Theme.cornerRadius
            readonly property real margin: Theme.space24
            readonly property real ratio: Math.max(0.0001, Math.min((width - 2 * margin) / root.spanX, (height - 2 * margin) / root.spanY))
            readonly property real originX: (width - root.spanX * ratio) / 2
            readonly property real originY: (height - root.spanY * ratio) / 2
            function outputAt(point) {
                var hit = root.rects.find(rect => !rect.asleep
                    && point.x >= originX + (rect.x - root.minX) * ratio && point.x < originX + (rect.x + rect.w - root.minX) * ratio
                    && point.y >= originY + (rect.y - root.minY) * ratio && point.y < originY + (rect.y + rect.h - root.minY) * ratio);
                return hit ? hit.name : '';
            }

            Repeater {
                model: root.rects
                delegate: Rectangle {
                    id: screen
                    required property var modelData
                    readonly property var rect: modelData
                    property real dragX: 0
                    property real dragY: 0
                    objectName: 'monitor-output-' + rect.name
                    x: arrangement.originX + (rect.x - root.minX) * arrangement.ratio + dragX
                    y: arrangement.originY + (rect.y - root.minY) * arrangement.ratio + dragY
                    z: move.pressed ? 2 : 1
                    width: rect.w * arrangement.ratio; height: rect.h * arrangement.ratio
                    radius: Theme.cornerRadius
                    color: rect.asleep ? Theme.base : root.selected && root.selected.name === rect.name ? Theme.selection : Theme.raised
                    border.width: root.selected && root.selected.name === rect.name ? 2 : 1
                    border.color: rect.asleep ? Theme.line : Theme.accent
                    opacity: rect.asleep ? 0.6 : 1
                    Accessible.name: rect.name
                    Accessible.role: Accessible.Button
                    activeFocusOnTab: !rect.asleep
                    Keys.onReturnPressed: root.selectedName = rect.name
                    Keys.onSpacePressed: root.selectedName = rect.name

                    MouseArea {
                        id: move
                        anchors.fill: parent
                        enabled: !screen.rect.asleep && !root.pending
                        cursorShape: root.outputs.length > 1 ? Qt.SizeAllCursor : Qt.ArrowCursor
                        property point start
                        onPressed: mouse => { start = mapToItem(arrangement, mouse.x, mouse.y); root.selectedName = screen.rect.name; }
                        onPositionChanged: mouse => {
                            if (root.outputs.length < 2) return;
                            var at = mapToItem(arrangement, mouse.x, mouse.y);
                            screen.dragX = at.x - start.x; screen.dragY = at.y - start.y;
                        }
                        onReleased: {
                            var moved = Math.abs(screen.dragX) + Math.abs(screen.dragY) > 4;
                            var x = screen.rect.x + screen.dragX / arrangement.ratio, y = screen.rect.y + screen.dragY / arrangement.ratio;
                            screen.dragX = 0; screen.dragY = 0;
                            if (moved) root.drop(screen.rect.name, x, y);
                        }
                        onCanceled: { screen.dragX = 0; screen.dragY = 0; }
                    }
                    ColumnLayout {
                        anchors.left: parent.left; anchors.top: parent.top; anchors.margins: Theme.space12
                        anchors.right: parent.right
                        spacing: 2
                        LabelText { text: screen.rect.name; font.weight: Font.Medium; elide: Text.ElideRight; Layout.fillWidth: true }
                        LabelText {
                            readonly property var draft: root.draftOutputs[screen.rect.name]
                            text: (draft ? draft.mode.split('@')[0] : screen.rect.w + 'x' + screen.rect.h).replace('x', '×')
                            color: Theme.muted; font.pixelSize: Theme.fontSizeSmall
                        }
                        LabelText { text: screen.rect.x + ', ' + screen.rect.y; color: Theme.muted; font.pixelSize: Theme.fontSizeSmall }
                        LabelText { visible: screen.rect.asleep; text: 'Asleep'; color: Theme.muted; font.pixelSize: Theme.fontSizeSmall }
                    }
                    Flow {
                        anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
                        anchors.margins: Theme.space12
                        spacing: Theme.space8
                        Repeater {
                            model: screen.rect.asleep
                                ? root.workspaceRows.filter(row => row.monitor === screen.rect.name).map(row => row.name)
                                : Object.keys(root.draftWorkspaces).filter(name => root.draftWorkspaces[name] === screen.rect.name)
                                    .sort((a, b) => (Number(a) || 0) - (Number(b) || 0) || a.localeCompare(b))
                            delegate: Rectangle {
                                id: chip
                                required property string modelData
                                property real dragX: 0
                                property real dragY: 0
                                objectName: 'monitor-chip-' + modelData
                                readonly property bool moved: { var row = root.workspaceRows.find(item => item.name === modelData); return !!row && row.monitor !== screen.rect.name; }
                                width: Math.max(36, chipLabel.implicitWidth + 2 * Theme.space12); height: 30
                                radius: Theme.cornerRadius
                                color: grab.pressed ? Theme.hover : Theme.panel
                                border.color: moved ? Theme.accent : Theme.edge
                                transform: Translate { x: chip.dragX; y: chip.dragY }
                                z: grab.pressed ? 3 : 0
                                Accessible.name: 'Workspace ' + modelData
                                Accessible.role: Accessible.Button
                                LabelText { id: chipLabel; anchors.centerIn: parent; text: chip.modelData }
                                MouseArea {
                                    id: grab
                                    anchors.fill: parent
                                    enabled: !screen.rect.asleep && !root.pending
                                    preventStealing: true
                                    property point start
                                    onPressed: mouse => start = mapToItem(arrangement, mouse.x, mouse.y)
                                    onPositionChanged: mouse => { var at = mapToItem(arrangement, mouse.x, mouse.y); chip.dragX = at.x - start.x; chip.dragY = at.y - start.y; }
                                    onReleased: mouse => {
                                        var dragged = Math.abs(chip.dragX) + Math.abs(chip.dragY) > 4;
                                        var at = mapToItem(arrangement, mouse.x, mouse.y);
                                        chip.dragX = 0; chip.dragY = 0;
                                        if (!dragged) { picker.workspace = chip.modelData; picker.openAt(chip); return; }
                                        var target = arrangement.outputAt(at);
                                        if (target) root.place(chip.modelData, target);
                                    }
                                    onCanceled: { chip.dragX = 0; chip.dragY = 0; }
                                }
                            }
                        }
                    }
                }
            }
        }

        ColumnLayout {
            Layout.preferredWidth: 260; Layout.maximumWidth: 260; Layout.fillHeight: true
            spacing: Theme.space8
            LabelText { text: root.selected ? root.selected.name : ''; font.pixelSize: Theme.sectionTitleSize; font.weight: Font.Medium; Layout.fillWidth: true; elide: Text.ElideRight }
            LabelText { text: 'Mode'; color: Theme.muted }
            Choice {
                objectName: 'monitor-mode'
                Layout.fillWidth: true
                enabled: !!root.selected && !root.pending
                model: root.selected ? root.selected.availableModes || [] : []
                displayText: currentText.replace(/Hz$/, ' Hz').replace('@', ' at ')
                currentIndex: root.selected && root.draftOutputs[root.selected.name] ? Math.max(0, model.indexOf(root.draftOutputs[root.selected.name].mode)) : 0
                onActivated: {
                    var scales = root.scalesFor(currentText);
                    var scale = String(root.draftOutputs[root.selected.name].scale);
                    root.updateOutput(root.selected.name, {mode: currentText, scale: scales.indexOf(scale) >= 0 ? Number(scale) : 1});
                }
            }
            LabelText { text: 'Scale'; color: Theme.muted }
            Choice {
                objectName: 'monitor-scale'
                Layout.fillWidth: true
                enabled: !!root.selected && !root.pending
                readonly property var draft: root.selected ? root.draftOutputs[root.selected.name] : null
                model: draft ? root.scalesFor(draft.mode, draft.scale) : []
                currentIndex: draft ? Math.max(0, model.indexOf(String(draft.scale))) : 0
                onActivated: root.updateOutput(root.selected.name, {scale: Number(currentText)})
            }
            LabelText { text: 'Orientation'; color: Theme.muted }
            Choice {
                objectName: 'monitor-orientation'
                Layout.fillWidth: true
                enabled: !!root.selected && !root.pending
                model: root.orientations
                currentIndex: root.selected && root.draftOutputs[root.selected.name] ? root.draftOutputs[root.selected.name].transform : 0
                onActivated: root.updateOutput(root.selected.name, {transform: currentIndex})
            }
            Item { Layout.preferredHeight: Theme.space16 }
            RowLayout {
                visible: root.strandedWindows > 0 || root.stranded.length > 0
                objectName: 'monitor-stranded'
                LabelText { text: 'Stranded'; color: Theme.dangerText }
                LabelText { text: String(root.strandedWindows); color: Theme.dangerText; font.weight: Font.Medium }
            }
            RowLayout {
                visible: !!root.fallback.canRecord
                spacing: Theme.space8
                SButton { objectName: 'monitor-record'; text: 'Record'; enabled: !root.busy && !root.pending; onClicked: recordDialog.open() }
                LabelText { visible: !!root.fallback.current; text: 'Recorded'; color: Theme.successText }
            }
            Item { Layout.fillHeight: true }
        }
    }

    RowLayout {
        Layout.fillWidth: true; Layout.preferredHeight: Theme.controlHeight; Layout.maximumHeight: Theme.controlHeight
        spacing: Theme.space8
        LabelText {
            objectName: 'monitor-summary'
            Layout.fillWidth: true; Layout.minimumWidth: 0
            elide: Text.ElideRight
            text: root.refusal || root.changes.join(' · ')
            color: root.refusal ? Theme.dangerText : Theme.text
        }
        SButton { objectName: 'monitor-reset'; text: 'Undo'; quiet: true; visible: root.changed && !root.pending; onClicked: root.resetDraft() }
        SButton { objectName: 'monitor-apply'; text: 'Apply'; primary: true; visible: root.changed && !root.pending; enabled: !!root.plan && !root.busy; busy: root.busy; onClicked: root.apply() }
        SButton { objectName: 'monitor-revert'; text: 'Undo ' + root.pendingSeconds; visible: !!root.pending; onClicked: root.confirm(false) }
        SButton { objectName: 'monitor-keep'; text: 'Keep'; primary: true; visible: !!root.pending; onClicked: root.confirm(true) }
    }

    SMenu {
        id: picker
        property string workspace: ''
        width: 180
        Repeater {
            model: root.outputNames
            delegate: MenuAction {
                required property string modelData
                objectName: 'monitor-pick-' + modelData
                text: modelData
                iconName: root.draftWorkspaces[picker.workspace] === modelData ? 'check' : ''
                onClicked: { root.place(picker.workspace, modelData); picker.close(); }
            }
        }
    }

    // Recording the one-output arrangement: the workspaces chosen to stay
    // first; every other workspace moves to the survivor when the state recurs.
    SDialog {
        id: recordDialog
        objectName: 'monitor-record-dialog'
        parent: Overlay.overlay; anchors.centerIn: parent; modal: true
        title: 'One screen'
        width: Math.min(420, parent ? parent.width - 32 : 420)
        property var stay: []
        onAboutToShow: stay = root.workspaceRows.filter(row => row.monitor === root.outputNames[0]).map(row => row.name)
        contentItem: ColumnLayout {
            spacing: Theme.space12
            LabelText { text: 'Stay'; color: Theme.muted }
            Flow {
                Layout.fillWidth: true; spacing: Theme.space8
                Repeater {
                    model: root.workspaceRows.map(row => row.name)
                    delegate: SCheckBox {
                        required property string modelData
                        objectName: 'monitor-stay-' + modelData
                        text: modelData
                        checked: recordDialog.stay.indexOf(modelData) >= 0
                        onToggled: recordDialog.stay = checked ? recordDialog.stay.concat([modelData]) : recordDialog.stay.filter(name => name !== modelData)
                    }
                }
            }
        }
        footer: RowLayout {
            Item { Layout.fillWidth: true }
            SButton {
                objectName: 'monitor-record-confirm'
                text: 'Record'; primary: true
                enabled: !root.busy
                Layout.margins: Theme.space12
                onClicked: {
                    root.busy = true;
                    Backend.send('desktop-fallback-record', {output: root.outputNames[0], stay: recordDialog.stay}, (result, ok, failure) => {
                        root.busy = false;
                        if (!ok) { root.error = failure || ''; return; }
                        recordDialog.close(); root.snapshotText = ''; root.load();
                    }, true);
                }
            }
        }
    }
}
