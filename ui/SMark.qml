import QtQuick
import QtQuick.Effects

// The real mark: the bar mark tinted to the text colour, or the primary
// circle as drawn.
Item {
    id: mark
    property bool primary: false
    property real size: Theme.chromeIconSize
    property color color: Theme.text
    implicitWidth: size
    implicitHeight: size
    Image {
        id: shape
        anchors.fill: parent
        visible: mark.primary
        source: mark.primary ? '../assets/soliton.png' : '../assets/soliton-monochrome.png'
        sourceSize: Qt.size(Math.ceil(mark.size * 2), Math.ceil(mark.size * 2))
        fillMode: Image.PreserveAspectFit
        mipmap: true
        smooth: true
    }
    MultiEffect {
        anchors.fill: parent
        source: shape
        visible: !mark.primary
        colorization: 1
        colorizationColor: mark.color
    }
    Accessible.role: Accessible.Graphic
    Accessible.name: 'Soliton'
}
