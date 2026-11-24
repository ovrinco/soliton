import QtQuick
import QtQuick.Controls
import "../ui" as Planning

Planning.SButton {
    property bool danger: false
    destructive: danger
}
