import QtQuick

Item {
    id: root
    property bool active: false
    property bool skills: false
    Loader {
        anchors.fill: parent
        sourceComponent: root.skills ? skillsPage : pluginsPage
    }
    Component { id: pluginsPage; PluginsPage { active: root.active } }
    Component { id: skillsPage; SkillsPage { active: root.active } }
}
