import QtQuick
import QtQuick.Window
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Quickshell
import Quickshell.Io
import "ui"
import "console" as Console

ShellRoot {
    FloatingWindow {
        id: window

        property bool plannerClosed: false
        property bool shuttingDown: false
        property real zoomScale: 1.0
        property var visitedPages: ({ "Dashboard": true })
        readonly property int toolWidth: { var w = Backend.prefs.paneWidths || {}; return Math.max(488, w.tool || w.console || 560); }

        function toggleNavigation() {
            Backend.navCollapsed = !Backend.navCollapsed;
            Backend.remember();
        }
        // Ctrl+J: the tools in the right slot in turn; the detail pane is not one.
        function cyclePane() { Backend.cycleTool(); }
        function isPageVisited(p) { return !!visitedPages[p]; }
        function markPageVisited(p) {
            if (!visitedPages[p]) {
                var next = Object.assign({}, visitedPages);
                next[p] = true;
                visitedPages = next;
            }
        }

        color: Theme.base
        implicitHeight: 1040
        implicitWidth: 1760
        minimumSize: Qt.size(640, 480)
        title: 'Soliton'

        onVisibleChanged: if (!visible && !shuttingDown) {
            visible = true;
            Backend.shutdown();
        }

        Connections {
            function onCloseReady() {
                Console.Backend.shutdown();
            }
            function onPaletteRequested() {
                palette.open();
            }
            function onPageChanged() {
                // navigate() snapshots the leaving scope before emitting this.
                // Existing destination scope is restored later in the same call;
                // a first visit must not inherit another page's task filters.
                if (Backend.restoringRoute) {
                    Backend.branches = []; Backend.search = '';
                    Backend.priority = 'all'; Backend.status = 'all'; Backend.showArchived = false;
                }
                window.markPageVisited(Backend.page);
            }
            // One selected horizon: a day, month or year opened from inside the
            // calendar becomes the Dashboard route, so its tab and the calendar
            // never disagree.
            function onCalendarModeChanged() {
                if (Backend.page === 'Dashboard' && !Backend.restoringRoute
                        && ['Day', 'Week', 'Month', 'Year'].indexOf(Backend.view) >= 0
                        && Backend.view !== Backend.calendarMode)
                    Backend.navigate('Dashboard', Backend.calendarMode);
            }

            target: Backend
        }
        Connections {
            function onShutdownReady() {
                Backend.send('shutdown', {}, () => {
                    window.shuttingDown = true;
                    Qt.quit();
                });
            }

            target: Console.Backend
        }
        Control {
            id: surface

            ProposalDialogs {}

            // Keep the logical layout in window coordinates while scaling the
            // whole surface from its top-left origin.  This lets the existing
            // keyboard zoom shortcuts affect the rendered UI without changing
            // the window size or breaking the overlay/dialog layer.
            x: 0
            y: 0
            width: parent.width / window.zoomScale
            height: parent.height / window.zoomScale
            scale: window.zoomScale
            transformOrigin: Item.TopLeft
            palette.base: Theme.panel
            palette.button: Theme.raised
            palette.buttonText: Theme.text
            palette.highlight: Theme.accent
            palette.highlightedText: Theme.accentForeground
            palette.placeholderText: Theme.muted
            palette.text: Theme.text
            palette.toolTipBase: Theme.panel
            palette.toolTipText: Theme.text
            palette.window: Theme.base
            palette.windowText: Theme.text

            background: Rectangle {
                color: Theme.base
            }

            ColumnLayout {
                id: mainContent
                objectName: "soliton-frame"

                anchors.fill: parent
                spacing: 0

                ShellHeader {
                    Layout.fillWidth: true
                    onPaletteRequested: seed => { palette.seed = seed; palette.open(); }
                }
                Rectangle {
                    Layout.fillWidth: true
                    color: Theme.line
                    implicitHeight: 1
                }
                RowLayout {
                    Layout.fillHeight: true
                    Layout.fillWidth: true
                    spacing: 0

                    ShellRail {
                        id: navigationRail
                        Layout.fillHeight: true
                        Layout.preferredWidth: railWidth
                        Layout.minimumWidth: railWidth
                        Layout.maximumWidth: railWidth
                        visible: !Backend.focusMode
                        onFoldRequested: window.toggleNavigation()
                    }
                    SidePanel {
                        id: sidePanel
                        Layout.fillHeight: true
                        Layout.preferredWidth: Theme.sidePanelWidth
                        Layout.minimumWidth: Theme.sidePanelWidth
                        Layout.maximumWidth: Theme.sidePanelWidth
                        visible: false
                    }
                    Rectangle {
                        Layout.fillHeight: true
                        color: Theme.line
                        implicitWidth: 1
                        visible: !Backend.focusMode
                    }
                    ColumnLayout {
                        id: content
                        Layout.minimumWidth: 0; Layout.minimumHeight: 0

                        readonly property bool compact: !Backend.focusMode && width < 388 + (Backend.hasRouteDetails ? 396 : 0) + (Backend.slotTool ? 496 : 0)

                        Layout.fillHeight: true
                        Layout.fillWidth: true
                        spacing: 0

                        SplitView {
                            id: split

                            Layout.fillHeight: true
                            Layout.fillWidth: true
                            orientation: Qt.Horizontal

                            handle: SSplitHandle { }

                            // An open side pane (detail, habit form, Hearth pane) is the pane
                            // that gives and takes width: the page keeps its own width, so the
                            // console seam is never pinned by the page's floor while the side
                            // pane still has room to give.
                            readonly property bool paneFills: detailContainer.visible || habitContainer.visible || hearthPaneContainer.visible
                            readonly property real paneWidth: Math.max(388, (Backend.prefs.paneWidths || {}).detail || 460)

                            onResizingChanged: if (!resizing && !content.compact && !Backend.detailFull) {
                                // Save only visible panes, once on release. A hidden
                                // pane's zero width must never overwrite its last size.
                                var widths = Object.assign({}, Backend.prefs.paneWidths || {});
                                if (detailContainer.visible) widths.detail = detailContainer.width;
                                // The tool slot keeps one width, whichever module it holds.
                                if (toolSlot.visible) widths.tool = toolSlot.width;
                                Backend.setting('paneWidths', widths);
                            }

                            Item {
                                id: work
                                objectName: "soliton-work"

                                SplitView.fillWidth: visible && !split.paneFills
                                SplitView.minimumWidth: content.compact ? 200 : 380
                                // Beside a filling side pane the page opens at the width it had
                                // when it filled: what the saved side pane and console leave.
                                SplitView.preferredWidth: Math.max(content.compact ? 200 : 380, split.width - split.paneWidth - (toolSlot.visible ? window.toolWidth + 6 : 0) - 6)
                                visible: !(Backend.hasRouteDetails && Backend.detailFull) && (!content.compact || !(Backend.hasRouteDetails && Backend.compactPane === 'detail' || Backend.slotModule && Backend.compactPane === 'console'))

                                ColumnLayout {
                                    anchors.fill: parent
                                    anchors.margins: surface.width < 760 ? 16 : Theme.pageInset
                                    // Keep the scroll edge close to an active splitter, rather
                                    // than leaving a full page inset between pointer and seam.
                                    anchors.rightMargin: detailContainer.visible || toolSlot.visible
                                        ? 4 : (surface.width < 760 ? 16 : Theme.pageInset)
                                    spacing: 12




                                    // A viewport, not the implicit width of a hidden route,
                                    // owns the work frame. Route loaders cannot resize the shell.
                                    Item {
                                        Layout.fillWidth: true; Layout.fillHeight: true
                                        Layout.minimumWidth: 0; Layout.minimumHeight: 0
                                    StackLayout {
                                        id: pageStack
                                        objectName: "soliton-page-stack"
                                        RetainedFocus { scope: pageStack; positionKey: Backend.page; active: work.visible }
                                        anchors.fill: parent
                                        currentIndex: {
                                            switch (Backend.page) {
                                                case 'Dashboard': case 'Dashboards': return 0;
                                                case 'Learning': case 'Learning Center': return 1;
                                                case 'Strategy': case 'Modules': return 2;
                                                case 'Goals': return 3;
                                                case 'Tasks': return 4;
                                                case 'Notes': return 5;
                                                case 'Portal': return 6;
                                                case 'Settings': return 7;
                                                case 'Hearth': return 8;
                                                default: return 0;
                                            }
                                        }

                                        Loader {
                                            active: window.isPageVisited('Dashboard') || Backend.page === 'Dashboard' || Backend.page === 'Dashboards'
                                            source: 'ui/Dashboard.qml'
                                        }
                                        Loader {
                                            active: window.isPageVisited('Learning') || Backend.page === 'Learning' || Backend.page === 'Learning Center'
                                            source: 'ui/IntelligenceView.qml'
                                        }
                                        Loader {
                                            active: window.isPageVisited('Strategy') || Backend.page === 'Strategy'
                                            source: 'ui/StrategyView.qml'
                                        }
                                        Loader {
                                            active: window.isPageVisited('Goals') || Backend.page === 'Goals' || Backend.view === 'Goals' || Backend.view === 'Categories & Goals'
                                            source: 'ui/GoalsView.qml'
                                        }
                                        Loader {
                                            active: window.isPageVisited('Tasks') || Backend.page === 'Tasks' || Backend.view === 'Tasks'
                                            source: 'ui/TaskView.qml'
                                        }
                                        Loader {
                                            active: window.isPageVisited('Notes') || Backend.page === 'Notes' || Backend.view === 'Notes'
                                            source: 'ui/DocumentsView.qml'
                                        }
                                        Loader {
                                            active: window.isPageVisited('Portal') || Backend.page === 'Portal'
                                            source: 'ui/PortalView.qml'
                                        }
                                        Loader {
                                            active: window.isPageVisited('Settings') || Backend.page === 'Settings'
                                            source: 'ui/SettingsView.qml'
                                        }
                                        ColumnLayout {
                                            spacing: 0
                                            STabBar {
                                                Layout.fillWidth: true
                                                Layout.leftMargin: Theme.pageInset; Layout.rightMargin: Theme.pageInset; Layout.topMargin: 12
                                                Repeater {
                                                    model: Backend.hearthViews
                                                    delegate: STabButton {
                                                        required property string modelData
                                                        text: modelData
                                                        active: (Backend.hearthView || 'Habits') === modelData
                                                        onClicked: Backend.navigate('Hearth', modelData)
                                                    }
                                                }
                                            }
                                            Loader {
                                                Layout.fillWidth: true; Layout.fillHeight: true
                                                active: window.isPageVisited('Hearth') || Backend.page === 'Hearth'
                                                source: Backend.hearthView === 'Kitchen' ? 'ui/KitchenView.qml'
                                                      : Backend.hearthView === 'Finance' ? 'ui/FinanceView.qml'
                                                      : Backend.hearthView === 'Now' ? 'ui/NowView.qml' : 'ui/HabitsView.qml'
                                            }
                                        }
                                    }
                                    }
                                }
                            }
                            Rectangle {
                                id: detailContainer
                                objectName: 'detail-slot'

                                SplitView.fillWidth: visible
                                SplitView.minimumWidth: content.compact ? 200 : 388
                                SplitView.preferredWidth: Backend.detailFull ? content.width : Math.max(388, (Backend.prefs.paneWidths || {}).detail || 460)
                                color: Theme.base
                                visible: Backend.hasRouteDetails && (!content.compact || Backend.compactPane === 'detail')

                                DetailPane {
                                    anchors.fill: parent
                                }
                            }
                            Rectangle {
                                id: habitContainer

                                SplitView.fillWidth: visible && !detailContainer.visible
                                SplitView.minimumWidth: content.compact ? 200 : 388
                                SplitView.preferredWidth: Math.max(388, (Backend.prefs.paneWidths || {}).detail || 460)
                                color: Theme.base
                                visible: Backend.page === 'Hearth' && Backend.view === 'Habits' && Backend.habitFormOpen && !Backend.focusMode

                                HabitForm {
                                    anchors.fill: parent
                                }
                            }
                            Rectangle {
                                id: hearthPaneContainer

                                SplitView.fillWidth: visible && !detailContainer.visible && !habitContainer.visible
                                SplitView.minimumWidth: content.compact ? 200 : 388
                                SplitView.preferredWidth: Math.max(388, (Backend.prefs.paneWidths || {}).detail || 460)
                                color: Theme.base
                                visible: Backend.page === 'Hearth' && Backend.view !== 'Habits' && !!Backend.hearthPane.kind && !Backend.focusMode

                                HearthPane {
                                    anchors.fill: parent
                                }
                            }
                            // The side panel: one module at a time, chosen from its launcher,
                            // cycled with Ctrl+J, or popped out into its own window.
                            Rectangle {
                                id: toolSlot
                                objectName: 'tool-slot'

                                SplitView.fillWidth: visible && !work.visible && !split.paneFills
                                SplitView.minimumWidth: content.compact ? 200 : 488
                                SplitView.preferredWidth: window.toolWidth
                                color: Theme.base
                                visible: !!Backend.slotModule && !Backend.focusMode && !Backend.detailFull && (!content.compact || Backend.compactPane === 'console')

                                Rectangle {
                                    id: slotHead
                                    objectName: 'tool-slot-head'
                                    anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
                                    height: 40
                                    color: Theme.panel
                                    RowLayout {
                                        anchors.fill: parent; anchors.leftMargin: 14; anchors.rightMargin: 8; spacing: 4
                                        LabelText {
                                            objectName: 'tool-slot-name'
                                            Layout.fillWidth: true
                                            text: Backend.moduleNames[Backend.slotModule] || ''
                                            font.weight: Font.DemiBold
                                        }
                                        SButton {
                                            objectName: 'tool-slot-tools'
                                            quiet: true
                                            text: 'Tools'
                                            visible: Backend.slotModule !== 'launcher'
                                            onClicked: Backend.openTool('launcher')
                                        }
                                        SButton {
                                            objectName: 'tool-slot-pop-out'
                                            quiet: true
                                            text: 'Pop out'
                                            visible: Backend.poppedTool === '' && Backend.slotModule !== 'launcher'
                                            onClicked: Backend.popOutTool()
                                        }
                                        SButton {
                                            objectName: 'tool-slot-close'
                                            quiet: true
                                            iconName: 'close'
                                            accessibleLabel: 'Close the side panel'
                                            onClicked: Backend.openTool('')
                                        }
                                    }
                                    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line }
                                }
                                Loader {
                                    anchors.fill: parent; anchors.topMargin: slotHead.height
                                    active: toolSlot.visible
                                    sourceComponent: Backend.slotModule === 'launcher' ? slotLauncher
                                        : Backend.slotModule ? slotTerminalTool : null
                                }
                                Component { id: slotLauncher; Loader { source: 'ui/ToolLauncher.qml' } }
                                Component {
                                    id: slotTerminalTool
                                    Console.ConsolePane {
                                        tool: Backend.slotModule
                                        onHideRequested: Backend.openTool('')
                                    }
                                }
                            }
                        }
                    }
                }

            }
            Rectangle {
                id: noticeOverlay
                objectName: 'notice-overlay'
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.margins: 16
                width: Math.min(520, parent.width-32)
                height: noticeRow.implicitHeight+24
                z: 500
                visible: !!Backend.notice && (Backend.noticeError || !/^(Your workspace is ready|Opening your workspace|Saved · |Completion updated|Occurrence rescheduled|Moved )/.test(Backend.notice))
                color: Theme.panel
                border.color: Backend.noticeError ? Theme.dangerText : Theme.edge
                Rectangle { width: 3; height: parent.height; color: Backend.noticeError ? Theme.dangerText : Theme.accent }
                RowLayout {
                    id: noticeRow
                    anchors.fill: parent; anchors.margins: 12; spacing: 12
                    LabelText {
                        Layout.fillWidth: true; text: Backend.notice
                        color: Backend.noticeError ? Theme.dangerText : Theme.text
                        wrapMode: Text.Wrap; elide: Text.ElideNone
                    }
                    SButton { iconName: 'close'; accessibleLabel: 'Dismiss notification'; quiet: true; onClicked: { Backend.notice = ''; Backend.noticeError = false; } }
                }
                Accessible.role: Accessible.AlertMessage
                Accessible.name: Backend.notice
            }

        }
        CommandPalette {
            id: palette
        }
        // A popped-out module in its own window; closing it returns nothing to the slot.
        FloatingWindow {
            id: toolWindow
            objectName: 'tool-window'
            visible: Backend.poppedTool !== ''
            implicitWidth: 760
            implicitHeight: 720
            color: Theme.base
            title: Backend.moduleNames[Backend.poppedTool] || 'Soliton'
            onVisibleChanged: if (!visible) {
                Backend.poppedTool = '';
                visible = Qt.binding(() => Backend.poppedTool !== '');
            }
            Item {
            id: toolContent
            anchors.fill: parent
            Loader {
                anchors.fill: parent
                active: Backend.terminalTools.indexOf(Backend.poppedTool) >= 0
                sourceComponent: Console.ConsolePane {
                    tool: Backend.poppedTool
                    onHideRequested: Backend.poppedTool = ''
                }
            }
            }
        }
        // The desktop's own file dialog: the only place a statement is named.
        FileDialog {
            id: statementPicker
            objectName: 'statement-picker'
            title: 'Choose a statement'
            nameFilters: ['CSV (*.csv)', 'All files (*)']
            onAccepted: {
                var path = String(selectedFile).replace(/^file:\/\//, '');
                try { path = decodeURIComponent(path); } catch (e) { }
                Backend.statementChosen(path);
            }
        }
        Connections {
            target: Backend
            function onStatementPickSerialChanged() { statementPicker.open(); }
        }
        Loader {
            active: Quickshell.env('SOLITON_AUTOMATION') === '1'
            source: 'tests/NativeInput.qml'

            onLoaded: item.surface = surface
        }
        Loader {
            active: Quickshell.env('SOLITON_DESIGN_PROBE') === '1'
            source: Quickshell.env('SOLITON_T01_PROBE') === '1' ? 'tests/T01/FrameProbe.qml' : 'tests/DesignFrameProbe.qml'
            onLoaded: item.surface = surface
        }
        Loader {
            active: Quickshell.env('SOLITON_T01_GALLERY') === '1'
            source: 'tests/T01/ControlGallery.qml'
        }
        Shortcut {
            sequence: SolitonKeys.chord('palette')

            onActivated: palette.open()
        }
        Shortcut {
            sequence: SolitonKeys.chord('rail')

            onActivated: window.toggleNavigation()
        }
        Shortcut {
            sequence: SolitonKeys.chord('pane')
            enabled: Theme.treatment === 2

            onActivated: window.cyclePane()
        }
        Shortcut {
            sequence: SolitonKeys.chord('task')

            // On Hearth Ctrl+N adds to the module shown, not a task.
            onActivated: Backend.page !== 'Hearth' ? Backend.create('task')
                         : Backend.view === 'Kitchen' ? Backend.kitchenAdd()
                         : Backend.view === 'Finance' ? Backend.chooseStatement()
                         : Backend.openHabitForm('')
        }
        Shortcut {
            sequence: SolitonKeys.chord('note')

            onActivated: Backend.create('note')
        }
        Shortcut {
            sequence: SolitonKeys.chord('save')

            onActivated: Backend.saveDraft()
        }
        Shortcut {
            sequence: SolitonKeys.chord('focus')

            onActivated: Backend.focusMode = !Backend.focusMode
        }
        Shortcut {
            sequence: SolitonKeys.chord('console')

            onActivated: Backend.openTool(Backend.consoleOpen ? '' : 'console')
        }
        Shortcut {
            sequence: SolitonKeys.chord('private')

            onActivated: Backend.togglePrivateMode()
            onActivatedAmbiguously: Backend.togglePrivateMode()
        }
        Shortcut {
            sequences: ['Ctrl+=', 'Ctrl+Shift+=', 'Ctrl++']
            onActivated: window.zoomScale = Math.min(1.8, Math.round((window.zoomScale + 0.1) * 10) / 10)
        }
        Shortcut {
            sequences: ['Ctrl+-', 'Ctrl+Shift+-']
            onActivated: window.zoomScale = Math.max(0.7, Math.round((window.zoomScale - 0.1) * 10) / 10)
        }
        Shortcut {
            sequences: ['Ctrl+0', 'Ctrl+Shift+0']
            onActivated: window.zoomScale = 1.0
        }
        IpcHandler {
            function capture(path: string) {
                var overlay = surface.Overlay.overlay;
                var original = overlay.parent;
                overlay.parent = surface;
                // Retain native scene capture at the actual window resolution,
                // including the application's zoom rather than logical pixels.
                surface.grabToImage(result => {
                    result.saveToFile(path);
                    overlay.parent = original;
                }, Qt.size(Math.round(window.width), Math.round(window.height)));
            }
            // Visible right-side tool panels in the split: one slot, so at most one.
            function rightToolPanels(): int {
                var n = 0, kids = split.contentChildren;
                for (var i = 0; i < kids.length; i++)
                    if (kids[i].visible && String(kids[i].objectName).indexOf('tool-slot') === 0) n++;
                return n;
            }
            // The slot's module name as the tree shows it, read from the head.
            function slotNameText(): string {
                var find = item => {
                    if (!item) return null;
                    if (item.objectName === 'tool-slot-name') return item;
                    for (var i = 0; i < item.children.length; i++) { var f = find(item.children[i]); if (f) return f; }
                    return null;
                };
                var label = toolSlot.visible ? find(toolSlot) : null;
                return label ? label.text : '';
            }
            // The launcher's entries as the tree shows them, in order.
            function launcherEntries(): string {
                var names = [];
                var walk = item => {
                    if (!item || !item.visible) return;
                    var n = String(item.objectName);
                    if (n.indexOf('tool-launcher-') === 0) names.push(n.slice(14));
                    for (var i = 0; i < item.children.length; i++) walk(item.children[i]);
                };
                walk(toolSlot);
                return JSON.stringify(names);
            }
            function captureTool(path: string) {
                toolContent.grabToImage(result => result.saveToFile(path));
            }
            function closeTool() {
                toolWindow.visible = false;
            }
            function close() {
                Backend.shutdown();
            }
            function inspect(): string {
                return JSON.stringify({
                    palette: palette.visible,
                    filters: false, // Filters now belong to their view.
                    saveView: false,
                    width: window.width,
                    height: window.height,
                    surfaceWidth: surface.width,
                    surfaceHeight: surface.height,
                    zoomScale: window.zoomScale,
                    compact: content.compact,
                    workVisible: work.visible,
                    detailVisible: detailContainer.visible,
                    consoleVisible: toolSlot.visible && Backend.consoleOpen,
                    slotTool: Backend.slotTool,
                    slotModuleName: slotNameText(),
                    rightToolPanels: rightToolPanels(),
                    poppedTool: Backend.poppedTool,
                    toolWindowVisible: toolWindow.visible,
                    toolWindowTitle: toolWindow.visible ? toolWindow.title : '',
                    navCollapsed: Backend.navCollapsed,
                    sidePanelVisible: sidePanel.visible,
                    focusItem: surface.activeFocus ? surface.objectName : ''
                });
            }
            function resize(width: int, height: int) {
                surface.Window.window.width = width;
                surface.Window.window.height = height;
            }
            function setScale(value: real) {
                window.zoomScale = Math.max(0.7, Math.min(1.8, value));
            }

            enabled: Quickshell.env('SOLITON_AUTOMATION') === '1'
            target: 'soliton-window'
        }
    }
}
