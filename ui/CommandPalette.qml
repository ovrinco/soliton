import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Routes.js" as Routes

SPopup {
    id: root
    width: Math.min(660, parent.width - 40)
    height: 460
    anchors.centerIn: parent
    // A click outside only closes the palette and reaches nothing underneath,
    // except the header's privacy control, which also toggles in that click.
    // The backdrop takes the click itself so that one control can be let through.
    modal: false
    dim: true
    closePolicy: Popup.CloseOnEscape
    Overlay.modeless: Rectangle {
        color: Qt.alpha(root.palette.shadow, 0.5)
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onPressed: mouse => {
                var control = root.privateControl();
                var onPrivate = false;
                if (control && control.visible) {
                    var p = mapToItem(control, mouse.x, mouse.y);
                    onPrivate = control.contains(p);
                }
                root.close();
                if (onPrivate && mouse.button === Qt.LeftButton)
                    Backend.togglePrivateMode();
            }
            onWheel: wheel => wheel.accepted = true
        }
    }
    function findNamed(item, name) {
        if (!item)
            return null;
        if (item.objectName === name)
            return item;
        for (var i = 0; i < item.children.length; i++) {
            var found = findNamed(item.children[i], name);
            if (found)
                return found;
        }
        return null;
    }
    property Item headerPrivate: null
    function privateControl() {
        if (!headerPrivate && root.Overlay.overlay && root.Overlay.overlay.parent)
            headerPrivate = findNamed(root.Overlay.overlay.parent, 'soliton-private');
        return headerPrivate;
    }
    focus: true
    padding: 18
    property var commands: [
        {
            title: 'Create task',
            hint: 'Ctrl+N',
            action: () => Backend.create('task')
        },
        {
            title: 'Create goal',
            hint: 'Dashboard',
            action: () => Backend.create('goal')
        },
        {
            title: 'Create category',
            hint: 'Organize your work',
            action: () => Backend.create('category')
        },
        {
            title: 'Quick note',
            hint: 'Ctrl+Shift+N',
            action: () => Backend.create('note')
        },
        {
            title: 'Plan today',
            hint: 'Day calendar',
            action: () => {
                Backend.selectedDate = Backend.today;
                Backend.calendarMode = 'Day';
                Backend.navigate('Dashboard');
            }
        },
        {
            title: 'Task board',
            hint: 'Status lanes',
            action: () => {
                Backend.board = true;
                Backend.navigate('Tasks');
            }
        },
        {
            title: 'Task list',
            hint: 'All your work',
            action: () => {
                Backend.board = false;
                Backend.navigate('Tasks');
            }
        },
        {
            // One row, two names: the word he uses for the bundles finds the page.
            title: 'Open habits',
            alias: 'Open bookends',
            hint: '',
            action: () => Backend.navigate('Hearth', 'Habits')
        },
        {
            title: 'Add habit',
            hint: 'Ctrl+N',
            action: () => Backend.openHabitForm('')
        },
        {
            title: 'Open kitchen',
            hint: '',
            action: () => Backend.navigate('Hearth', 'Kitchen')
        },
        {
            title: 'Open finance',
            hint: '',
            action: () => Backend.navigate('Hearth', 'Finance')
        },
        {
            title: 'Open now',
            hint: '',
            action: () => Backend.navigate('Hearth', 'Now')
        },
        {
            title: 'Add to stock',
            hint: '',
            action: () => Backend.openHearthPane('newLot')
        },
        {
            title: 'Plan a batch',
            hint: '',
            when: () => Backend.kitchenStock.length > 0,
            action: () => Backend.openHearthPane('plan')
        },
        {
            title: 'Import statement',
            hint: '',
            action: () => Backend.chooseStatement()
        },
        {
            title: 'Check in: morning',
            hint: '',
            action: () => {
                Backend.navigate('Hearth', 'Habits');
                Backend.habitFocusRequest = {serial: Backend.habitFocusRequest.serial + 1, bundle: 'morning'};
            }
        },
        {
            title: 'Check in: evening',
            hint: '',
            action: () => {
                Backend.navigate('Hearth', 'Habits');
                Backend.habitFocusRequest = {serial: Backend.habitFocusRequest.serial + 1, bundle: 'evening'};
            }
        },
        {
            title: 'Play or pause',
            hint: 'Space',
            when: () => Backend.voicePlaybackReady,
            action: () => Backend.voicePlayback('toggle')
        },
        {
            title: 'Slower',
            hint: '[',
            when: () => Backend.voicePlaybackReady,
            action: () => Backend.voicePlayback('slower')
        },
        {
            title: 'Faster',
            hint: ']',
            when: () => Backend.voicePlaybackReady,
            action: () => Backend.voicePlayback('faster')
        },
        {
            title: 'Normal speed',
            hint: '0',
            when: () => Backend.voicePlaybackReady,
            action: () => Backend.voicePlayback('normal')
        },
        {
            title: 'Categories & goals',
            hint: 'Dashboard',
            action: () => Backend.navigate('Goals')
        },
        {
            title: 'Private',
            hint: 'Ctrl+Shift+P',
            action: () => Backend.togglePrivateMode()
        },
        {
            title: Backend.focusMode ? 'Exit focus' : 'Focus workspace',
            hint: 'Ctrl+Shift+F',
            action: () => Backend.focusMode = !Backend.focusMode
        },
        {
            title: Backend.consoleOpen ? 'Close console' : 'Open console',
            hint: 'Ctrl+Shift+A',
            action: () => {
                Backend.consoleOpen = !Backend.consoleOpen;
                Backend.compactPane = 'console';
            }
        },
        {
            title: 'Clear filters',
            hint: 'Show all work',
            action: () => Backend.clearFilters()
        },
        {
            title: 'Show archived work',
            hint: 'Restore or inspect',
            action: () => {
                Backend.navigate('Tasks');
                Backend.clearFilters();
                Backend.showArchived = true;
                Backend.closeDetails();
            }
        }
    ]
    // One row per side-panel view: "<Page> · <View>", navigating as the row does.
    readonly property var viewCommands: {
        var rows = [];
        var add = (page, word, views, run) => views.forEach(v => rows.push({title: word + ' · ' + v, hint: '', action: () => run(v)}));
        add('Dashboard', 'Dashboard', ['Overview', 'Day', 'Week', 'Month', 'Year'], v => {
            var route = v === 'Overview' ? 'Timeline' : v;
            Backend.navigate('Dashboard', route);
            if (route === v) Backend.calendarMode = v;
        });
        add('Tasks', 'Tasks', ['List', 'Board'], v => { Backend.navigate('Tasks'); Backend.board = v === 'Board'; Backend.remember(); });
        add('Goals', 'Categories & goals', ['Cards', 'List'], v => { Backend.navigate('Goals'); Backend.goalList = v === 'List'; Backend.remember(); });
        add('Goals', 'Categories & goals', ['Archive'], v => { Backend.navigate('Goals'); Backend.showArchived = !Backend.showArchived; Backend.remember(); });
        add('Notes', 'Notes', ['Notes'], v => Backend.navigate('Notes', 'Notes'));
        ['Learning', 'Strategy', 'Portal', 'Settings'].forEach(page => add(page, page, Routes.pageViews(page), v => {
            if (page === 'Strategy') { Backend.strategyTab = v; Backend.navigate(page, v); Backend.remember(); }
            else Backend.navigate(page, v);
        }));
        return rows;
    }
    function named(c) {
        var q = query.text.toLowerCase();
        return !q || c.title.toLowerCase().includes(q) || !c.alias ? c.title
            : c.alias.toLowerCase().includes(q) ? c.alias : c.title;
    }
    property var choices: commands.concat(viewCommands).filter(c => !c.when || c.when()).filter(c => !query.text || [c.title, c.alias || ''].some(t => t.toLowerCase().includes(query.text.toLowerCase()))).map(c => Object.assign({}, c, {title: named(c)})).concat(Backend.records.filter(r => !r.hidden && query.text && r.title.toLowerCase().includes(query.text.toLowerCase())).slice(0, 20).map(r => ({
                title: r.title,
                hint: r.kind,
                objectId: r.id
            })))
    background: Rectangle {
        color: Theme.panel
        border.color: Theme.line
        radius: Theme.cornerRadius
    }
    // Text typed into a header command bar before the palette opened.
    property string seed: ''
    onOpened: {
        query.text = seed;
        seed = '';
        query.forceActiveFocus();
        results.currentIndex = 0;
    }
    function run(index) {
        var choice = choices[index];
        if (!choice)
            return;
        close();
        if (choice.objectId)
            Backend.select(choice.objectId);
        else
            choice.action();
    }
    ColumnLayout {
        anchors.fill: parent
        spacing: 12
        RowLayout {
            Layout.fillWidth: true
            LabelText {
                text: 'Go somewhere. Do something.'
                font.pixelSize: 16
                Layout.fillWidth: true
            }
            LabelText {
                text: 'ESC'
                font.pixelSize: 12
                color: Theme.muted
            }
        }
        Field {
            id: query
            Layout.fillWidth: true
            placeholderText: 'Search commands, tasks, goals and notes…'
            onTextChanged: results.currentIndex = 0
            onAccepted: root.run(results.currentIndex)
            Keys.onDownPressed: results.currentIndex = Math.min(results.count - 1, results.currentIndex + 1)
            Keys.onUpPressed: results.currentIndex = Math.max(0, results.currentIndex - 1)
        }
        ListView {
            id: results
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: root.choices
            spacing: 3
            delegate: Rectangle {
                id: row
                required property var modelData
                required property int index
                width: results.width
                height: 43
                radius: Theme.cornerRadius
                color: results.currentIndex === index ? Theme.selection : 'transparent'
                RowLayout {
                    anchors.fill: parent
                    anchors.margins: 10
                    LabelText {
                        text: row.modelData.title
                        Layout.fillWidth: true
                    }
                    LabelText {
                        text: row.modelData.hint
                        font.pixelSize: 12
                        color: Theme.muted
                    }
                }
                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    onEntered: results.currentIndex = row.index
                    onClicked: root.run(row.index)
                }
            }
            ScrollBar.vertical: SScrollBar {}
        }
    }
}
