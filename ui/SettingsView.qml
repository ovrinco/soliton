pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
ColumnLayout {
    id: root
    spacing: 12
    readonly property string activeTab: Backend.page === 'Settings' ? (Backend.view === 'Plugins & Skills' ? 'Plugins' : Backend.view) : 'General'
    readonly property var tabs: ['General', 'Monitors', 'Key bindings', 'Plugins', 'Skills', 'Zeno', 'Log', 'Processes']
    STabBar {
        visible: Theme.pageTabs
        Layout.fillWidth: true
        Repeater {
            model: root.tabs
            delegate: STabButton {
                required property string modelData
                text: modelData
                active: root.activeTab === modelData
                onClicked: Backend.navigate('Settings', modelData)
            }
        }
        Item { Layout.fillWidth: true }
    }
    StackLayout {
        Layout.minimumWidth: 0; Layout.minimumHeight: 0
        Layout.fillWidth: true
        Layout.fillHeight: true
        currentIndex: root.activeTab === 'Recovery' ? root.tabs.length : Math.max(0, root.tabs.indexOf(root.activeTab))
        ControlView { onRecoveryRequested: Backend.navigate('Settings', 'Recovery') }
        SettingsDevicePane { monitors: true; active: root.visible && root.activeTab === 'Monitors' }
        KeyBindingsView { active: root.visible && root.activeTab === 'Key bindings' }
        SettingsInventory { active: root.visible && root.activeTab === 'Plugins' }
        SettingsInventory { skills: true; active: root.visible && root.activeTab === 'Skills' }
        // Loaded by file: the page is not in the folder's type list.
        Loader {
            id: zenoPage
            source: 'SettingsZeno.qml'
            Binding { target: zenoPage.item; property: 'active'; value: root.visible && root.activeTab === 'Zeno'; when: zenoPage.status === Loader.Ready }
        }
        // The machine's log and its processes, loaded by file and read only while their tab shows.
        Loader {
            id: logPage
            source: 'MachineLogView.qml'
            Binding { target: logPage.item; property: 'active'; value: root.visible && root.activeTab === 'Log'; when: logPage.status === Loader.Ready }
        }
        Loader {
            id: processesPage
            source: 'ProcessesView.qml'
            Binding { target: processesPage.item; property: 'active'; value: root.visible && root.activeTab === 'Processes'; when: processesPage.status === Loader.Ready }
        }
        ColumnLayout {
            id: recoveryPage
            property bool project: true
            SButton { text: 'Back to General'; onClicked: Backend.navigate('Settings', 'General') }
            RowLayout {
                SButton { text: 'Project checkpoints'; primary: recoveryPage.project; onClicked: recoveryPage.project = true }
                SButton { text: 'Individual files'; primary: !recoveryPage.project; onClicked: recoveryPage.project = false }
            }
            Loader { Layout.fillWidth: true; Layout.fillHeight: true; sourceComponent: recoveryPage.project ? projectRecovery : fileRecovery }
            Component { id: projectRecovery; ProjectRecoveryView {} }
            Component { id: fileRecovery; RecoveryView {} }
        }
    }
}
