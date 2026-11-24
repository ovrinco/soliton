import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

SButton {
    id: action
    quiet: true
    implicitHeight: Theme.controlHeight
    Layout.fillWidth: true
    contentItem: RowLayout {
        spacing: Theme.controlSpacing
        SIcon { visible: !!action.iconName; name: action.iconName; color: action.ink }
        LabelText { Layout.fillWidth: true; text: action.text; color: action.ink; font.pixelSize: Theme.fontSizeNormal }
    }
}
