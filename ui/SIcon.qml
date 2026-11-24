import QtQuick
import QtQuick.Effects

// One icon family: Lucide (ISC), shipped under assets/icons at stroke 1.5.
// Every name maps to exactly one glyph; the build refuses a name that is not
// in a table below (build.rs), so there is no fallback drawing.
Item {
    id: icon
    property string name: ''
    property color color: Theme.text
    property real strokeWidth: Theme.iconStroke
    property real size: Theme.chromeIconSize
    implicitWidth: size
    implicitHeight: size

    readonly property var glyphs: ({
        'dashboard': 'layout-grid',
        'learning': 'book-open',
        'strategy': 'compass',
        'goals': 'target',
        'tasks': 'square-check-big',
        'notes': 'file-text',
        'file': 'file-text',
        'portal': 'arrow-left-right',
        'settings': 'sliders-horizontal',
        'filter': 'list-filter',
        'search': 'search',
        'expand': 'maximize-2',
        'dock': 'minimize-2',
        'copy': 'copy',
        'edit': 'pencil',
        'list': 'list',
        'board': 'columns-3',
        'close': 'x',
        'left': 'chevron-left',
        'right': 'chevron-right',
        'down': 'chevron-down',
        'check': 'check',
        'select': 'square-check',
        'play': 'play',
        'pause': 'pause',
        'console': 'square-terminal',
        'disk': 'folder',
        'folder': 'folder',
        'warning': 'triangle-alert',
        'clock': 'clock',
        'more': 'ellipsis',
        'plus': 'plus',
        'source': 'code-xml',
    })
    readonly property var added: ({
        'private': 'eye-off',
        'hearth': 'flame',
        'calendar': 'calendar',
    })
    readonly property string glyph: glyphs[name.toLowerCase()] || added[name.toLowerCase()] || ''
    onGlyphChanged: if (name && !glyph) console.error('SIcon: no glyph for', name)

    Image {
        id: shape
        anchors.fill: parent
        visible: false
        source: icon.glyph ? Qt.resolvedUrl('../assets/icons/' + icon.glyph + '.svg') : ''
        sourceSize: Qt.size(Math.ceil(icon.width * 2), Math.ceil(icon.height * 2))
        fillMode: Image.PreserveAspectFit
        smooth: true
    }
    MultiEffect {
        anchors.fill: parent
        source: shape
        visible: !!icon.glyph
        colorization: 1
        colorizationColor: icon.color
    }
    Accessible.ignored: true
}
