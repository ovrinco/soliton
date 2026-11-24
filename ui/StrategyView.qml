pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Presentation.js" as Presentation

ColumnLayout {
    id: root
    spacing: 12

    // Create each Strategy surface only when first visited, then retain the
    // visual instance. This keeps cold entry cheap while preserving an
    // in-progress proposal when the user switches tabs.
    property var visitedTabs: [true, false, false, false, false]
    readonly property var tabNames: ['Overview', 'Ideas', 'Execution', 'Tracker', 'Chats', 'Voice notes', 'Runs', 'Modules']

    property string activeTab: {
        if (Backend.view === 'History') return Presentation.historyTab(Backend.prefs.historyKind);
        if (root.tabNames.indexOf(Backend.view) >= 0)
            return Backend.view;
        var tab = Backend.strategyTab;
        if (tab === 'Stack Overview') return 'Overview';
        if (tab === 'Ideation') return 'Ideas';
        if (tab === 'Proposals') return 'Execution';
        if (root.tabNames.indexOf(tab) >= 0) return tab;
        return 'Overview';
    }

    function markVisited(tab) {
        var index = Math.min(4, root.tabNames.indexOf(tab));
        if (tab === 'Modules' || index < 0 || root.visitedTabs[index])
            return;
        var next = root.visitedTabs.slice(0);
        next[index] = true;
        root.visitedTabs = next;
    }

    onActiveTabChanged: root.markVisited(root.activeTab)

    Component.onCompleted: root.markVisited(root.activeTab)

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.space8

        STabBar {
            visible: Theme.pageTabs
            Layout.fillWidth: true

            Repeater {
                model: root.tabNames
                delegate: STabButton {
                    required property string modelData
                    text: modelData
                    active: root.activeTab === modelData
                    onClicked: {
                        Backend.strategyTab = modelData;
                        Backend.navigate('Strategy', modelData);
                        root.markVisited(modelData);
                        Backend.remember();
                    }
                }
            }
            Item { Layout.fillWidth: true }
        }

        SButton {
            text: 'Resume proposal'
            visible: !!Backend.proposalDraft.source_id
            onClicked: Backend.composeProposalRequested()
        }
    }

    StackLayout {
        Layout.minimumWidth: 0; Layout.minimumHeight: 0
        Layout.fillWidth: true
        Layout.fillHeight: true
        currentIndex: root.activeTab === 'Modules' ? 5 : Math.max(0, Math.min(4, root.tabNames.indexOf(root.activeTab)))

        // Tab 0: Stack Overview (4-layer OVRIN Cube)
        Loader {
            asynchronous: true
            Layout.fillWidth: true
            Layout.fillHeight: true
            active: root.visitedTabs[0]
            // The one module the stack still holds opens where it lives:
            // Modules › Installed modules.
            sourceComponent: StackMap {
                onOpenModule: function(id) {
                    Backend.developerModule = id;
                    Backend.developerSection = 'Modules';
                    Backend.strategyTab = 'Modules';
                    Backend.navigate('Strategy', 'Modules');
                    Backend.remember();
                }
            }
        }

        // Ideas are real persistent strategy documents, not a module launcher.
        Loader {
            asynchronous: true
            Layout.fillWidth: true
            Layout.fillHeight: true
            active: root.visitedTabs[1]
            sourceComponent: DocumentsView { kind: 'strategy'; embedded: true }
        }

        // Execution uses the durable reviewed proposal owner.
        Loader {
            asynchronous: true
            Layout.fillWidth: true
            Layout.fillHeight: true
            active: root.visitedTabs[2]
            sourceComponent: ProposalsView {}
        }

        // Tracker: the sheet as it stands in the tracker store, in the thin grid.
        Loader {
            asynchronous: true
            Layout.fillWidth: true
            Layout.fillHeight: true
            active: root.visitedTabs[3]
            sourceComponent: TrackerView { active: root.activeTab === 'Tracker' }
        }

        // Tab 4: History (Audit trail, transcripts & voice notes)
        Loader {
            asynchronous: true
            Layout.fillWidth: true
            Layout.fillHeight: true
            active: root.visitedTabs[4]
            sourceComponent: HistoryView {
                requestedKind: Presentation.historyKind(root.activeTab)
                onRecordKindResolved: function(kind) {
                    if (Backend.view === 'History') {
                        Backend.strategyTab = Presentation.historyTab(kind);
                        Backend.navigate('Strategy', Backend.strategyTab);
                    }
                }
                onProposalRequested: function(record, segment) {
                    Backend.beginProposal(record, segment);
                }
            }
        }
        // Modules: the developer tools, kept once opened.
        Loader {
            objectName: "strategy-modules"
            Layout.fillWidth: true
            Layout.fillHeight: true
            active: root.activeTab === 'Modules' || status === Loader.Ready
            sourceComponent: DeveloperView {}
        }
    }
}
