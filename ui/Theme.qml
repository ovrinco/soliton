pragma Singleton
import QtQuick
import Quickshell
import Quickshell.Io
import Qt.labs.folderlistmodel

QtObject {
    id: theme

    readonly property color accent: colors.accent || '#d9a93c'
    // Accent surfaces vary with the active Omarchy palette.  Pick the
    // palette foreground with the strongest contrast instead of assuming
    // either a dark or light theme.
    readonly property color accentForeground: foregroundFor(accent)
    readonly property int cornerRadius: 2
    readonly property int controlHeight: 36
    readonly property int denseRowHeight: 34
    readonly property int twoLineRowHeight: 52
    readonly property int tabStripHeight: 40
    readonly property int iconTargetSize: 34
    readonly property int space4: 4
    readonly property int space8: 8
    readonly property int space12: 12
    readonly property int space16: 16
    readonly property int space24: 24
    readonly property int fontSizeNormal: 14
    readonly property int fontSizeSecondary: 12
    readonly property int fontSizeSmall: 12
    readonly property color base: colors.background || '#121214'
    property var colors: ({})
    readonly property color danger: colors.red || '#d15f4e'
    readonly property string font: 'Noto Sans'
    readonly property color green: colors.green || '#7f8f6a'
    readonly property color hover: Qt.tint(raised, Qt.alpha(text, .07))
    readonly property bool light: colors.mode === 'light'
    readonly property color line: Qt.tint(base, Qt.alpha(text, .16))
    readonly property string mono: 'JetBrains Mono'
    readonly property color muted: readableAcross(colors.muted || colors.dark_foreground || '#8e8e93', [base, panel, raised, selection])
    property FileView palette: FileView {
        path: theme.palettePath
        preload: true
        printErrors: false
        watchChanges: true

        onFileChanged: reload()
        onLoaded: theme.apply(text())
    }
    // The saved Settings choice, set by Backend from the workspace preferences:
    // '' (none saved) is the OVRIN palette, 'desktop' follows Omarchy's current
    // theme, any other value names an installed Omarchy theme.
    property string choice: ''
    readonly property string configThemes: (Quickshell.env('XDG_CONFIG_HOME') || Quickshell.env('HOME') + '/.config') + '/omarchy/themes'
    readonly property string systemThemes: (Quickshell.env('OMARCHY_PATH') || '/usr/share/omarchy') + '/themes'
    readonly property string desktopPalette: (Quickshell.env('XDG_STATE_HOME') || Quickshell.env('HOME') + '/.local/state') + '/omarchy/current/theme/colors.toml'
    readonly property bool followsDesktop: !Quickshell.env('SOLITON_THEME_FILE') && choice === 'desktop'
    readonly property string palettePath: Quickshell.env('SOLITON_THEME_FILE') || themePath(choice)
    // Installed selectable themes: the user's own themes, then Omarchy's.
    property var themes: []
    property FolderListModel userThemeDirs: FolderListModel {
        folder: 'file://' + theme.configThemes
        showFiles: false
        onStatusChanged: theme.listThemes()
        onCountChanged: theme.listThemes()
    }
    property FolderListModel systemThemeDirs: FolderListModel {
        folder: 'file://' + theme.systemThemes
        showFiles: false
        onStatusChanged: theme.listThemes()
        onCountChanged: theme.listThemes()
    }
    function names(model) {
        var list = [];
        for (var i = 0; i < model.count; i++)
            list.push(String(model.get(i, 'fileName')));
        return list;
    }
    function listThemes() {
        var user = names(userThemeDirs), all = user.slice();
        names(systemThemeDirs).forEach(n => { if (all.indexOf(n) < 0) all.push(n); });
        all.sort();
        themes = all.map(n => ({ id: n, user: user.indexOf(n) >= 0 }));
    }
    function themePath(name) {
        if (name === 'desktop')
            return desktopPalette;
        var id = name || 'ovrin';
        var entry = themes.find(t => t.id === id);
        return (entry && !entry.user ? systemThemes : configThemes) + '/' + id + '/colors.toml';
    }
    readonly property color panel: colors.dark_background || '#0b0b0d'
    readonly property color raised: colors.lighter_background || '#1f1f24'
    readonly property color selection: colors.selection || '#33301f'
    readonly property color text: colors.foreground || '#e3e1dc'
    // File watchers can lose their inode when Omarchy swaps its current
    // theme directory. Only that followed file is re-read; a chosen theme's
    // own file is stable and its watcher's change events suffice.
    property Timer themeRefresh: Timer {
        interval: 2500
        repeat: true
        running: theme.followsDesktop

        onTriggered: theme.palette.reload()
    }

    // Content-led workbench: one scale governs chrome, rows and readers.
    // Preserve cornerRadius on the accepted planner; new chrome is squared.
    readonly property int chromeRadius: 2
    // Window frame: the activity rail with the side panel. It is the only
    // frame; the environment does not choose another.
    readonly property int treatment: 2
    readonly property int shellHeight: 36
    readonly property int railWidth: 200
    readonly property int railFoldedWidth: 44
    readonly property bool railText: true
    readonly property int railItemHeight: 36
    readonly property int railTextSize: 14
    readonly property int sidePanelWidth: 220
    // Each page's own views are tabs at its top; the left panel holds the pages only.
    readonly property bool pageTabs: true
    readonly property bool commandBar: false
    readonly property int commandBarWidth: 420
    readonly property int chromeIconSize: 16
    readonly property int railIconSize: 18
    readonly property real iconStroke: 1.5
    readonly property int actionHeight: 40
    readonly property int pageInset: 20
    readonly property int pageTitleSize: 30
    readonly property int sectionTitleSize: 18
    readonly property color fieldSurface: Qt.tint(base, Qt.alpha(text, .025))
    readonly property color fieldLine: Qt.tint(base, Qt.alpha(text, .24))
    readonly property int readerSize: 17
    readonly property int readerMeasure: 720
    readonly property int titleSize: 22
    readonly property color edge: Qt.tint(base, Qt.alpha(text, .28))
    readonly property color wash: Qt.tint(base, Qt.alpha(accent, .08))
    readonly property color accentText: readable(accent, base)
    readonly property color dangerText: readableAcross(danger, [base, panel, raised])
    readonly property color successText: readableAcross(green, [base, panel, raised])
    readonly property bool reducedMotion: Quickshell.env('SOLITON_REDUCED_MOTION') !== '0'
    readonly property int feedbackDuration: reducedMotion ? 0 : 90

    // One control convention. Every shared control reads its size, spacing
    // and colour here; no control or module carries a scheme of its own.
    // Buttons, dropdowns and inputs share one height and one inner padding.
    readonly property int buttonMinWidth: 64
    readonly property int iconButtonSize: controlHeight
    // A button nested inside an input (clear, calendar) keeps the field's edge.
    readonly property int innerButtonSize: controlHeight - space4
    readonly property int controlPadding: space12
    readonly property int controlSpacing: space8
    // An input's text area; a field's nested buttons add their own width.
    readonly property int inputWidth: 160
    readonly property int indicatorSize: 16
    readonly property int sliderHandleSize: 22
    readonly property color clear: 'transparent'
    // Every text-on-fill pair of a control meets this ratio in every state.
    readonly property real readableFloor: 4.5
    // Roles: primary, secondary and danger, solid; quiet and danger-quiet
    // are their borderless forms; danger-secondary is danger text on the
    // secondary fill.
    readonly property color buttonPrimaryFill: readableFill(accent)
    readonly property color buttonPrimaryText: foregroundFor(buttonPrimaryFill)
    readonly property color buttonSecondaryFill: raised
    readonly property color buttonSecondaryText: readableAcross(text, [raised, hover, selection, base, panel])
    readonly property color buttonDangerFill: readableFill(danger)
    readonly property color buttonDangerText: foregroundFor(buttonDangerFill)
    readonly property color buttonDangerInk: readableAcross(danger, [base, panel, raised, hover, selection])
    readonly property color buttonDisabledFill: panel
    readonly property color buttonDisabledText: muted
    readonly property color buttonBorder: line
    readonly property color inputFill: fieldSurface
    readonly property color inputText: readableAcross(text, [fieldSurface, hover, panel])
    readonly property color inputPlaceholder: readableAcross(muted, [fieldSurface, panel])
    readonly property color tabHover: Qt.alpha(text, .04)
    readonly property color scrim: Qt.alpha(base, .48)

    function buttonRole(primary, destructive, quiet) {
        if (destructive)
            return primary ? 'danger' : quiet ? 'danger-quiet' : 'danger-secondary';
        return primary ? 'primary' : quiet ? 'quiet' : 'secondary';
    }
    function buttonFill(role, state) {
        var solid = role === 'primary' || role === 'danger';
        if (state === 'disabled')
            return role === 'quiet' || role === 'danger-quiet' ? clear : buttonDisabledFill;
        if (solid) {
            var fill = role === 'danger' ? buttonDangerFill : buttonPrimaryFill;
            return state === 'pressed' ? shaded(fill, .16) : state === 'hover' ? shaded(fill, .08) : fill;
        }
        if (state === 'pressed') return selection;
        if (state === 'hover') return hover;
        return role === 'quiet' || role === 'danger-quiet' ? clear : buttonSecondaryFill;
    }
    function buttonInk(role, state) {
        if (state === 'disabled') return buttonDisabledText;
        if (role === 'primary') return buttonPrimaryText;
        if (role === 'danger') return buttonDangerText;
        if (role === 'danger-secondary' || role === 'danger-quiet') return buttonDangerInk;
        return buttonSecondaryText;
    }
    function buttonEdge(role) {
        return role === 'secondary' || role === 'danger-secondary' ? buttonBorder : clear;
    }
    // A solid fill that some palette foreground reads on at the floor.
    function readableFill(value) {
        var c = Qt.tint(value, 'transparent'), ink = foregroundFor(c);
        var away = luminance(ink) < .5 ? Qt.rgba(1, 1, 1, 1) : Qt.rgba(0, 0, 0, 1);
        for (var step = 0; step <= 20; ++step) {
            var candidate = mixed(c, away, step / 20);
            if (contrast(candidate, ink) >= readableFloor)
                return candidate;
        }
        return away;
    }
    // Hover and press shade a solid fill toward its text, unless that would
    // drop below the floor; then they shade away from it.
    function shaded(fill, amount) {
        var ink = foregroundFor(fill), toward = mixed(fill, ink, amount);
        if (contrast(toward, ink) >= readableFloor)
            return toward;
        return mixed(fill, luminance(ink) < .5 ? Qt.rgba(1, 1, 1, 1) : Qt.rgba(0, 0, 0, 1), amount);
    }

    function mixed(a, b, amount) {
        a = Qt.tint(a, 'transparent'); b = Qt.tint(b, 'transparent');
        return Qt.rgba(a.r + (b.r-a.r)*amount, a.g + (b.g-a.g)*amount,
                       a.b + (b.b-a.b)*amount, 1);
    }
    function readable(value, background) {
        return readableAcross(value, [background]);
    }
    function readableAcross(value, backgrounds) {
        var c = Qt.tint(value, 'transparent');
        var target = foregroundFor(backgrounds[0]);
        for (var step = 0; step <= 20; ++step) {
            var candidate = mixed(c, target, step / 20);
            if (backgrounds.every(function(bg) { return contrast(bg, candidate) >= 4.5; }))
                return candidate;
        }
        return target;
    }

    function apply(raw) {
        var next = {}, pattern = /^\s*([a-z_]+)\s*=\s*["'](#[0-9a-fA-F]{6}|dark|light)["']/gm, match;
        while ((match = pattern.exec(raw)) !== null)
            next[match[1]] = match[2];
        // Never flash to defaults during an atomic theme-directory replacement.
        if (next.background && next.foreground && JSON.stringify(next) !== JSON.stringify(colors))
            colors = next;
    }

    function channel(value) {
        return value <= 0.03928 ? value / 12.92 : Math.pow((value + 0.055) / 1.055, 2.4);
    }

    function luminance(value) {
        return 0.2126 * channel(value.r) + 0.7152 * channel(value.g) + 0.0722 * channel(value.b);
    }

    function contrast(background, foreground) {
        var a = luminance(background) + 0.05;
        var b = luminance(foreground) + 0.05;
        return Math.max(a, b) / Math.min(a, b);
    }

    function foregroundFor(background) {
        // Theme.base and Theme.text are the semantic surface/text pair for
        // the active palette. Include stable dark/light fallbacks so a
        // palette with two light semantic colours still gets a readable
        // accent foreground.
        var candidates = [base, text, Qt.rgba(17 / 255, 18 / 255, 20 / 255, 1), Qt.rgba(247 / 255, 247 / 255, 247 / 255, 1)];
        var best = candidates[0], bestRatio = contrast(background, best);
        for (var i = 1; i < candidates.length; ++i) {
            var ratio = contrast(background, candidates[i]);
            if (ratio > bestRatio) {
                best = candidates[i];
                bestRatio = ratio;
            }
        }
        return best;
    }
}
