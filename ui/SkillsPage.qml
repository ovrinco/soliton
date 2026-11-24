pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The skill manager (firstpick.skill-manager) inside Soliton's frame: its
// views, cards, selection and its select, choose tools, review, apply
// workflow, on its own engine through the settings bridge.
Item {
    id: root
    objectName: 'skills-page'
    property bool active: false
    property bool busy: false
    property bool loaded: false
    property string view: 'all'
    property string dialog: ''
    property string query: ''
    property var inventory: ({skills: [], harnesses: [], sources: [], restrictions: [], notes: []})
    property var selected: ({})
    property var targetTools: []
    property var preview: null
    property string token: ''
    property var detail: null
    property var validation: null
    property string importPath: ''
    property string message: ''
    property bool messageError: false

    readonly property var skills: inventory.skills || []
    readonly property var harnesses: inventory.harnesses || []
    readonly property var librarySkills: skills.filter(s => s.library)
    readonly property var rows: view === 'library' ? librarySkills : skills
    readonly property var filtered: rows.filter(function(s) {
        var words = [s.name, s.description, s.linkedNames.join(' '), s.found.join(' '), s.source, s.collections.join(' ')].join(' ');
        return words.toLowerCase().indexOf(query.toLowerCase()) >= 0;
    })
    readonly property var selectedRows: rows.filter(s => selected[s.id] === true)
    readonly property bool listView: view === 'all' || view === 'library'
    readonly property var viewWords: ({all: 'All skills', library: 'Library', scope: 'Scope & sources', validate: 'Validate', restrictions: 'Restrictions', create: 'New skill'})

    function call(request, done) {
        busy = true;
        SystemSettings.request({action: 'skills', request: request}, (result, failure) => {
            busy = false; loaded = true;
            if (failure) { messageError = true; message = failure; return; }
            messageError = false;
            if (result.skills) inventory = result;
            if (done) done(result);
        });
    }
    function load(words) {
        preview = null; token = ''; selected = ({});
        call({action: 'list'}, () => {
            message = words || 'Scan complete. Nothing was changed.';
            if (!targetTools.length) targetTools = harnesses.map(h => h.key);
        });
    }
    function navigate(next) {
        if (busy || preview) return;
        view = next; dialog = ''; query = ''; search.text = ''; selected = ({});
    }
    function pick(row, checked) {
        if (busy) return;
        if (row.errors.length) { showDetails(row); return; }
        var next = Object.assign({}, selected);
        if (checked) next[row.id] = true; else delete next[row.id];
        selected = next;
    }
    function selectAll() {
        var next = ({});
        filtered.forEach(s => { if (!s.errors.length) next[s.id] = true; });
        selected = next;
    }
    function showDetails(row) { if (!busy && !preview) { detail = row; dialog = 'details'; } }
    function beginShare(importing) {
        if (busy || (!importing && !selectedRows.length)) return;
        if (!targetTools.length) targetTools = harnesses.map(h => h.key);
        dialog = importing ? 'import' : 'share';
    }
    function toggleTool(key) {
        var next = targetTools.slice(), at = next.indexOf(key);
        if (at >= 0) next.splice(at, 1); else next.push(key);
        targetTools = next;
    }
    function requestPreview(request) {
        preview = null; token = '';
        request.action = 'preview';
        call(request, result => { preview = result.preview; token = result.token; message = 'Not applied yet.'; });
    }
    function submitShare() {
        if (dialog === 'import') requestPreview({operation: 'share', path: importPath, tools: targetTools});
        else requestPreview({operation: 'share', ids: selectedRows.map(s => s.id), tools: targetTools});
    }
    function libraryAction(operation) {
        requestPreview({operation: operation, names: selectedRows.map(s => s.name)});
    }
    function approve() {
        if (busy || !token) return;
        var approval = token; token = '';
        call({action: 'apply', token: approval}, result => {
            var made = result.applied.changes.filter(c => c.kind === 'link').length;
            var removed = result.applied.changes.filter(c => c.kind === 'unlink').length;
            preview = null; dialog = ''; selected = ({});
            message = result.applied.status !== 'committed' ? 'No changes were needed.'
                : 'Changes applied' + (made ? ': ' + made + (made === 1 ? ' link made' : ' links made') : '')
                  + (removed ? (made ? ', ' : ': ') + removed + (removed === 1 ? ' link removed' : ' links removed') : '') + '.';
        });
    }
    function cancelDialog() {
        if (busy) return;
        var had = !!preview || token !== '';
        preview = null; token = ''; dialog = '';
        if (had) { SystemSettings.request({action: 'skills', request: {action: 'cancel'}}, () => {}); message = 'Review cancelled. Nothing was changed.'; }
    }
    function toolName(key) { var h = harnesses.find(x => x.key === key); return h ? h.name : key; }

    // Scanned afresh each time the page is opened, unless a change is under review;
    // there is no rescan button.
    onActiveChanged: if (active && !busy && (!loaded || !preview)) load()
    Shortcut { enabled: root.active && root.listView && !root.dialog && !root.preview; sequence: 'Ctrl+F'; onActivated: search.forceActiveFocus() }
    Shortcut { enabled: root.active && (!!root.dialog || !!root.preview); sequence: 'Escape'; onActivated: root.cancelDialog() }

    component Words: Text {
        textFormat: Text.PlainText
        color: Theme.text
        font.family: Theme.mono
        font.pixelSize: 13
        wrapMode: Text.WordWrap
    }
    component Quiet: Words { color: Theme.muted; font.pixelSize: 12 }
    component Head: Words { color: Theme.accentText; font.weight: Font.DemiBold }

    // Main surface: title block, search, views, list, status bar.
    ColumnLayout {
        anchors.fill: parent
        spacing: 14
        visible: !root.dialog && !root.preview
        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            ColumnLayout {
                spacing: 4
                Words { objectName: 'skills-title'; text: 'Skills'; font.pixelSize: 25; font.weight: Font.Bold; wrapMode: Text.NoWrap }
                Words { text: 'SETTINGS  /  SKILLS  /  ' + root.viewWords[root.view].toUpperCase(); color: Theme.accentText; font.pixelSize: 11; wrapMode: Text.NoWrap }
            }
            Item { Layout.fillWidth: true }
            ManagerButton { objectName: 'skills-link-from-path'; text: 'Link from path…'; enabled: !root.busy; onClicked: root.beginShare(true) }
            ManagerButton { objectName: 'skills-new'; text: '+ New skill'; selected: root.view === 'create'; enabled: !root.busy; onClicked: root.navigate('create') }
        }
        Field {
            id: search
            objectName: 'skills-search'
            Layout.fillWidth: true
            visible: root.listView
            enabled: !root.busy
            font.family: Theme.mono
            font.pixelSize: 13
            placeholderText: 'Search skills by name, description, tool or source…'
            Accessible.name: 'Search skills'
            onTextEdited: { root.query = text; root.selected = ({}); }
            Keys.onEscapePressed: focus = false
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 16
            ColumnLayout {
                objectName: 'skills-views'
                Layout.preferredWidth: 205; Layout.maximumWidth: 205
                Layout.fillHeight: true
                spacing: 6
                ManagerButton { objectName: 'skills-view-all'; Layout.fillWidth: true; leftAlign: true; text: 'All skills  ·  ' + root.skills.length; selected: root.view === 'all'; enabled: !root.busy; onClicked: root.navigate('all') }
                ManagerButton { objectName: 'skills-view-library'; Layout.fillWidth: true; leftAlign: true; text: 'Library  ·  ' + root.librarySkills.length; selected: root.view === 'library'; enabled: !root.busy; onClicked: root.navigate('library') }
                Item { Layout.preferredHeight: 14 }
                ManagerButton { objectName: 'skills-view-scope'; Layout.fillWidth: true; leftAlign: true; text: 'Scope & sources'; selected: root.view === 'scope'; enabled: !root.busy; onClicked: root.navigate('scope') }
                ManagerButton { objectName: 'skills-view-validate'; Layout.fillWidth: true; leftAlign: true; text: 'Validate'; selected: root.view === 'validate'; enabled: !root.busy; onClicked: root.navigate('validate') }
                ManagerButton { objectName: 'skills-view-restrictions'; Layout.fillWidth: true; leftAlign: true; text: 'Restrictions'; selected: root.view === 'restrictions'; enabled: !root.busy; onClicked: root.navigate('restrictions') }
                Item { Layout.fillHeight: true }
                Words { text: 'USER-WIDE'; color: Theme.accentText; font.pixelSize: 11; font.weight: Font.Bold }
                Quiet { Layout.fillWidth: true; text: 'Applies across projects' }
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.minimumWidth: 0

                // All skills and Library: the cards.
                ColumnLayout {
                    anchors.fill: parent
                    visible: root.listView
                    spacing: 10
                    RowLayout {
                        Layout.fillWidth: true
                        Head { Layout.fillWidth: true; text: root.viewWords[root.view] + '  ·  ' + root.filtered.length; wrapMode: Text.NoWrap }
                        ManagerButton { objectName: 'skills-select-results'; text: 'Select results'; enabled: !root.busy && root.filtered.some(s => !s.errors.length); onClicked: root.selectAll() }
                        ManagerButton { objectName: 'skills-clear-selection'; text: 'Clear selection'; visible: root.selectedRows.length > 0; enabled: !root.busy; onClicked: root.selected = ({}) }
                    }
                    Flow {
                        objectName: 'skills-harnesses'
                        Layout.fillWidth: true
                        spacing: 6
                        Quiet { text: 'Linked in'; height: 26; verticalAlignment: Text.AlignVCenter; wrapMode: Text.NoWrap }
                        Repeater {
                            model: root.harnesses
                            delegate: ManagerBadge {
                                required property var modelData
                                objectName: 'harness-' + modelData.key
                                text: modelData.name + '  ' + modelData.linked
                                quiet: modelData.linked === 0
                            }
                        }
                    }
                    ListView {
                        id: cards
                        objectName: 'skills-list'
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        spacing: 6
                        boundsBehavior: Flickable.StopAtBounds
                        model: root.filtered
                        ScrollBar.vertical: SScrollBar {}
                        delegate: Rectangle {
                            id: card
                            required property var modelData
                            readonly property bool chosen: root.selected[modelData.id] === true
                            readonly property bool invalid: modelData.errors.length > 0
                            readonly property bool linked: modelData.linked.length > 0
                            readonly property bool attention: modelData.issues.length > 0 || ['source changed', 'source renamed', 'needs attention'].indexOf(modelData.integrity) >= 0
                            objectName: 'skill-card-' + modelData.name
                            width: cards.width - 12
                            height: content.implicitHeight + 22
                            radius: Theme.chromeRadius
                            color: chosen ? Theme.raised : mouse.containsMouse || activeFocus ? Theme.hover : Theme.fieldSurface
                            border.width: activeFocus ? 2 : 1
                            border.color: invalid || attention ? Theme.danger : chosen || activeFocus || linked ? Theme.accent : Theme.line
                            activeFocusOnTab: !root.busy
                            Accessible.role: Accessible.CheckBox
                            Accessible.name: modelData.name
                            Accessible.checked: chosen
                            Keys.onSpacePressed: root.pick(modelData, !chosen)
                            Keys.onReturnPressed: root.showDetails(modelData)
                            MouseArea {
                                id: mouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                enabled: !root.busy
                                onClicked: { card.forceActiveFocus(); root.pick(card.modelData, !card.chosen); }
                                onDoubleClicked: root.showDetails(card.modelData)
                            }
                            ColumnLayout {
                                id: content
                                anchors { left: parent.left; right: parent.right; top: parent.top; margins: 11 }
                                spacing: 5
                                RowLayout {
                                    Layout.fillWidth: true
                                    spacing: 8
                                    Words { text: card.chosen ? '☑' : '☐'; color: Theme.accentText; wrapMode: Text.NoWrap }
                                    Words { Layout.fillWidth: true; text: card.modelData.name; font.weight: Font.Bold; elide: Text.ElideRight; wrapMode: Text.NoWrap }
                                    ManagerBadge { text: 'LINKED'; visible: card.linked }
                                    Quiet {
                                        text: card.invalid ? 'CHECK FAILED' : card.attention ? String(card.modelData.integrity || 'needs attention').toUpperCase()
                                            : root.view === 'library' && !card.linked ? 'NOT LINKED' : ''
                                        visible: text !== ''
                                        color: card.invalid || card.attention ? Theme.dangerText : Theme.muted
                                        wrapMode: Text.NoWrap
                                    }
                                }
                                Flow {
                                    Layout.fillWidth: true
                                    visible: card.linked
                                    spacing: 6
                                    Words { text: 'Linked to:'; color: Theme.accentText; font.weight: Font.Bold; height: 26; verticalAlignment: Text.AlignVCenter; wrapMode: Text.NoWrap }
                                    Repeater {
                                        model: card.modelData.linkedNames
                                        delegate: ManagerBadge { required property string modelData; text: modelData }
                                    }
                                }
                                Flow {
                                    Layout.fillWidth: true
                                    visible: card.modelData.issues.length > 0
                                    spacing: 6
                                    Repeater {
                                        model: card.modelData.issues
                                        delegate: ManagerBadge { required property string modelData; text: modelData + ' · link needs attention'; warning: true }
                                    }
                                }
                                Quiet {
                                    Layout.fillWidth: true
                                    visible: card.modelData.found.length > 0
                                    text: 'Found for: ' + card.modelData.found.join(', ') + ' · discovered, not linked'
                                }
                                Words {
                                    Layout.fillWidth: true
                                    text: card.invalid ? card.modelData.errors.join('; ') : card.modelData.description
                                    color: card.invalid ? Theme.dangerText : Theme.muted
                                    maximumLineCount: 2
                                    elide: Text.ElideRight
                                }
                                Words {
                                    Layout.fillWidth: true
                                    text: card.modelData.source
                                    color: card.linked ? Theme.accentText : Theme.muted
                                    font.pixelSize: 12
                                    font.weight: card.linked ? Font.DemiBold : Font.Normal
                                    elide: Text.ElideRight
                                    wrapMode: Text.NoWrap
                                }
                            }
                        }
                        Quiet {
                            anchors.centerIn: parent
                            width: parent.width - 40
                            horizontalAlignment: Text.AlignHCenter
                            visible: root.loaded && cards.count === 0
                            text: root.rows.length ? 'No skill matches this search.' : root.view === 'library' ? 'The skills home holds no skill.' : 'No skills were found.'
                        }
                    }
                }

                // Scope & sources: what the engine scanned, named by collection.
                ScrollView {
                    anchors.fill: parent
                    visible: root.view === 'scope'
                    contentWidth: availableWidth
                    clip: true
                    ColumnLayout {
                        width: parent.width
                        spacing: 14
                        Head { text: 'Scope & sources' }
                        Words { text: 'Scope'; font.weight: Font.Bold }
                        Quiet { text: root.inventory.scope || 'User-wide · all projects' }
                        Words { text: 'Collections scanned'; font.weight: Font.Bold }
                        Repeater {
                            model: root.inventory.sources || []
                            delegate: RowLayout {
                                id: sourceRow
                                required property var modelData
                                Layout.fillWidth: true
                                Layout.maximumWidth: 520
                                Words { Layout.fillWidth: true; text: sourceRow.modelData.name; wrapMode: Text.NoWrap }
                                Quiet { text: sourceRow.modelData.count + (sourceRow.modelData.count === 1 ? ' skill' : ' skills') }
                            }
                        }
                        Words { text: 'Coding tools found'; font.weight: Font.Bold }
                        Repeater {
                            model: root.harnesses
                            delegate: RowLayout {
                                id: toolRow
                                required property var modelData
                                Layout.fillWidth: true
                                Layout.maximumWidth: 520
                                Words { Layout.fillWidth: true; text: toolRow.modelData.name; wrapMode: Text.NoWrap }
                                Quiet { text: toolRow.modelData.linked + (toolRow.modelData.linked === 1 ? ' link' : ' links') }
                            }
                        }
                        Words { text: 'Discovery notes'; font.weight: Font.Bold; visible: (root.inventory.notes || []).length > 0 }
                        Quiet { Layout.fillWidth: true; text: (root.inventory.notes || []).join('\n\n'); visible: text !== '' }
                    }
                }

                // Validate: the engine's local checks of one skill.
                ColumnLayout {
                    anchors.fill: parent
                    visible: root.view === 'validate'
                    spacing: 14
                    Head { text: 'Validate a skill' }
                    Words { text: 'Skill' }
                    Choice {
                        id: validateChoice
                        objectName: 'skills-validate-choice'
                        Layout.fillWidth: true
                        Layout.maximumWidth: 520
                        font.family: Theme.mono
                        model: root.skills.map(s => s.name + (s.library ? '' : '  ·  ' + s.source))
                        enabled: !root.busy
                        onActivated: root.validation = null
                        Accessible.name: 'Skill to validate'
                    }
                    ManagerButton {
                        objectName: 'skills-validate'
                        text: 'Validate'
                        primary: true
                        enabled: !root.busy && root.skills.length > 0
                        onClicked: root.call({action: 'validate', id: root.skills[validateChoice.currentIndex].id}, result => {
                            root.validation = result.validation;
                            root.message = result.validation.errors.length ? 'Local checks found problems.' : 'Local checks passed.';
                            root.messageError = result.validation.errors.length > 0;
                        })
                    }
                    Words { objectName: 'skills-validation-name'; Layout.fillWidth: true; visible: !!root.validation; text: root.validation ? root.validation.name : ''; font.weight: Font.Bold }
                    Words {
                        Layout.fillWidth: true
                        visible: !!root.validation
                        text: !root.validation ? '' : root.validation.errors.length ? root.validation.errors.join('\n\n') : 'Local checks passed. Runtime compatibility has not been tested.'
                        color: root.validation && root.validation.errors.length ? Theme.dangerText : Theme.accentText
                    }
                    Quiet { Layout.fillWidth: true; visible: !!root.validation; text: root.validation ? root.validation.warnings.join('\n\n') : '' }
                    Item { Layout.fillHeight: true }
                }

                // Restrictions & recovery: the engine's settings changes.
                ScrollView {
                    anchors.fill: parent
                    visible: root.view === 'restrictions'
                    contentWidth: availableWidth
                    clip: true
                    ColumnLayout {
                        width: Math.min(parent.width, 620)
                        spacing: 14
                        Head { text: 'Restrictions & recovery' }
                        Words { text: 'Restrict a skill in one tool'; font.weight: Font.Bold }
                        Choice {
                            id: restrictSkill
                            objectName: 'skills-restrict-skill'
                            Layout.fillWidth: true
                            font.family: Theme.mono
                            model: Array.from(new Set(root.skills.map(s => s.name)))
                            enabled: !root.busy
                            Accessible.name: 'Skill to restrict'
                        }
                        Choice {
                            id: restrictTool
                            objectName: 'skills-restrict-tool'
                            Layout.fillWidth: true
                            font.family: Theme.mono
                            textRole: 'name'
                            model: root.harnesses.filter(h => h.key !== 'cursor')
                            enabled: !root.busy
                            Accessible.name: 'Coding tool'
                        }
                        ManagerButton {
                            text: 'Preview restriction'
                            enabled: !root.busy && restrictSkill.count > 0 && restrictTool.count > 0
                            onClicked: root.requestPreview({operation: 'disable', name: restrictSkill.currentText, tool: restrictTool.model[restrictTool.currentIndex].key})
                        }
                        Words { text: 'Undo a recorded restriction'; font.weight: Font.Bold }
                        Choice {
                            id: restoreChoice
                            Layout.fillWidth: true
                            font.family: Theme.mono
                            visible: count > 0
                            model: (root.inventory.restrictions || []).map(op => op.tool + ' · ' + op.names.join(', '))
                            enabled: !root.busy
                            Accessible.name: 'Recorded restriction'
                        }
                        Quiet { visible: restoreChoice.count === 0; text: 'No restriction recorded.' }
                        ManagerButton {
                            text: 'Preview settings restore'
                            visible: restoreChoice.count > 0
                            enabled: !root.busy
                            onClicked: root.requestPreview({operation: 'enable', operationId: root.inventory.restrictions[restoreChoice.currentIndex].id})
                        }
                        Words { text: 'Interrupted operation'; font.weight: Font.Bold }
                        ManagerButton { text: 'Preview recovery'; enabled: !root.busy; onClicked: root.requestPreview({operation: 'recover'}) }
                    }
                }

                // New skill: a new original in the skills home.
                ColumnLayout {
                    anchors.fill: parent
                    visible: root.view === 'create'
                    spacing: 14
                    Head { text: 'New skill' }
                    Words { text: 'Skill name' }
                    Field { id: createName; objectName: 'skills-create-name'; Layout.fillWidth: true; Layout.maximumWidth: 620; font.family: Theme.mono; enabled: !root.busy; Accessible.name: 'Skill name' }
                    Words { text: 'Description' }
                    Field { id: createDescription; objectName: 'skills-create-description'; Layout.fillWidth: true; Layout.maximumWidth: 620; font.family: Theme.mono; maximumLength: 1024; enabled: !root.busy; Accessible.name: 'Description' }
                    ManagerButton {
                        text: 'Preview creation'
                        primary: true
                        enabled: !root.busy && createName.text.trim() !== '' && createDescription.text.trim() !== ''
                        onClicked: root.requestPreview({operation: 'create', name: createName.text.trim(), description: createDescription.text})
                    }
                    Item { Layout.fillHeight: true }
                }
            }
        }

        // Status bar: the last result and the bulk actions.
        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            Words {
                objectName: 'skills-status'
                Layout.fillWidth: true
                text: root.busy ? 'Working…' : root.selectedRows.length && root.listView ? root.selectedRows.length + ' selected · selection alone changes nothing' : root.message
                color: root.messageError ? Theme.dangerText : root.selectedRows.length && root.listView ? Theme.accentText : Theme.muted
                font.pixelSize: 12
                maximumLineCount: 2
                elide: Text.ElideRight
            }
            ManagerButton { objectName: 'skills-details'; text: 'Details…'; visible: root.listView && root.selectedRows.length === 1; enabled: !root.busy; onClicked: root.showDetails(root.selectedRows[0]) }
            ManagerButton { objectName: 'skills-recheck'; text: 'Recheck links…'; visible: root.view === 'library' && root.selectedRows.some(s => s.managed.length); enabled: !root.busy; onClicked: root.libraryAction('update') }
            ManagerButton { objectName: 'skills-unlink'; text: 'Unlink…'; danger: true; visible: root.view === 'library' && root.selectedRows.some(s => s.managed.length); enabled: !root.busy && root.selectedRows.every(s => s.managed.length); onClicked: root.libraryAction('remove') }
            ManagerButton { objectName: 'skills-link-selected'; text: root.view === 'library' ? 'Link to tools…' : 'Link selected…'; visible: root.listView; primary: true; enabled: !root.busy && root.selectedRows.length > 0; onClicked: root.beginShare(false) }
        }
    }

    // Choose tools (Link selected… or Link from path…).
    ColumnLayout {
        objectName: 'skills-share'
        anchors.fill: parent
        spacing: 14
        visible: !root.preview && (root.dialog === 'share' || root.dialog === 'import')
        RowLayout {
            Layout.fillWidth: true
            Words { Layout.fillWidth: true; text: root.dialog === 'import' ? 'Link a local skill' : 'Link selected skills'; font.pixelSize: 24; font.weight: Font.Bold }
            ManagerButton { text: 'Cancel'; enabled: !root.busy; onClicked: root.cancelDialog() }
        }
        Head { text: 'User-wide · all projects'; font.weight: Font.Normal }
        Quiet { Layout.fillWidth: true; visible: root.dialog === 'share'; text: root.selectedRows.map(s => s.name).join(', '); font.pixelSize: 13 }
        ColumnLayout {
            Layout.fillWidth: true
            visible: root.dialog === 'import'
            Words { text: 'Skill folder' }
            Field { objectName: 'skills-import-path'; Layout.fillWidth: true; Layout.maximumWidth: 720; font.family: Theme.mono; text: root.importPath; enabled: !root.busy; onTextEdited: root.importPath = text; Accessible.name: 'Skill folder' }
        }
        Words { text: 'Create links for'; font.weight: Font.Bold }
        Flow {
            Layout.fillWidth: true
            spacing: 7
            Repeater {
                model: root.harnesses
                delegate: ManagerButton {
                    required property var modelData
                    readonly property bool on: root.targetTools.indexOf(modelData.key) >= 0
                    objectName: 'skills-tool-' + modelData.key
                    text: (on ? '✓  ' : '') + modelData.name
                    selected: on
                    enabled: !root.busy
                    Accessible.role: Accessible.CheckBox
                    Accessible.checked: on
                    onClicked: root.toggleTool(modelData.key)
                }
            }
        }
        Item { Layout.fillHeight: true }
        RowLayout {
            Layout.fillWidth: true
            Quiet { Layout.fillWidth: true; text: root.targetTools.length + ' of ' + root.harnesses.length + ' tools selected' }
            ManagerButton {
                objectName: 'skills-review'
                text: 'Review changes'
                primary: true
                enabled: !root.busy && root.targetTools.length > 0 && (root.dialog === 'import' ? root.importPath.trim() !== '' : root.selectedRows.length > 0)
                onClicked: root.submitShare()
            }
        }
        Words { Layout.fillWidth: true; visible: root.messageError; text: root.message; color: Theme.dangerText; font.pixelSize: 12 }
    }

    // Review changes: the exact links the engine will make or remove.
    ColumnLayout {
        objectName: 'skills-preview'
        anchors.fill: parent
        spacing: 14
        visible: !!root.preview
        RowLayout {
            Layout.fillWidth: true
            Words { Layout.fillWidth: true; text: root.preview ? root.preview.title : ''; font.pixelSize: 24; font.weight: Font.Bold }
            ManagerButton { text: 'Cancel'; enabled: !root.busy; onClicked: root.cancelDialog() }
        }
        Head {
            objectName: 'skills-preview-count'
            font.weight: Font.Normal
            text: root.preview ? root.preview.changes.length + (root.preview.changes.length === 1 ? ' change' : ' changes') + '  ·  User-wide  ·  Not applied' : ''
        }
        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: availableWidth
            clip: true
            Column {
                width: parent.width
                spacing: 7
                Quiet { width: parent.width; visible: !!root.preview && root.preview.changes.length === 0; text: 'No changes are needed.'; font.pixelSize: 13 }
                Words { width: parent.width; visible: !!root.preview && root.preview.compatibility.length > 0; text: 'Potential compatibility issues'; color: Theme.dangerText; font.weight: Font.Bold }
                Quiet { width: parent.width; visible: !!root.preview && root.preview.compatibility.length > 0; text: root.preview ? root.preview.compatibility.join('\n\n') : ''; font.pixelSize: 13 }
                Repeater {
                    model: root.preview ? root.preview.changes : []
                    delegate: Rectangle {
                        id: change
                        required property var modelData
                        objectName: 'skills-change-' + modelData.kind + '-' + modelData.skill + '-' + modelData.tool
                        width: parent.width
                        height: changeText.implicitHeight + 22
                        radius: Theme.chromeRadius
                        color: Theme.fieldSurface
                        RowLayout {
                            id: changeText
                            anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter; margins: 11 }
                            spacing: 14
                            Words { text: change.modelData.kind.toUpperCase(); color: Theme.accentText; Layout.preferredWidth: 70; wrapMode: Text.NoWrap }
                            Words { Layout.fillWidth: true; text: change.modelData.text }
                        }
                    }
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Quiet { Layout.fillWidth: true; text: root.messageError ? root.message : 'Only this exact review is applied. It expires in five minutes.'; color: root.messageError ? Theme.dangerText : Theme.muted }
            ManagerButton { text: 'Cancel'; enabled: !root.busy; onClicked: root.cancelDialog() }
            ManagerButton {
                objectName: 'skills-apply'
                text: root.preview && root.preview.changes.length ? 'Apply changes' : 'Nothing to apply'
                primary: true
                enabled: !root.busy && root.token !== '' && !!root.preview && root.preview.changes.length > 0
                onClicked: root.approve()
            }
        }
    }

    // Details of one skill.
    ColumnLayout {
        objectName: 'skills-details-view'
        anchors.fill: parent
        spacing: 14
        visible: !root.preview && root.dialog === 'details'
        RowLayout {
            Layout.fillWidth: true
            Words { Layout.fillWidth: true; text: root.detail ? root.detail.name : ''; font.pixelSize: 24; font.weight: Font.Bold }
            ManagerButton { text: 'Back'; onClicked: root.cancelDialog() }
        }
        Words { Layout.fillWidth: true; text: root.detail ? root.detail.description : '' }
        Words { text: 'Source'; font.weight: Font.Bold }
        Head { Layout.fillWidth: true; font.weight: Font.Normal; text: root.detail ? root.detail.collections.join(', ') : '' }
        Quiet { Layout.fillWidth: true; text: root.detail ? 'Linked to: ' + (root.detail.linkedNames.join(', ') || 'no tool') : '' }
        Quiet { Layout.fillWidth: true; visible: !!root.detail && root.detail.found.length > 0; text: root.detail ? 'Found for: ' + root.detail.found.join(', ') : '' }
        Words { Layout.fillWidth: true; visible: !!root.detail && root.detail.errors.length > 0; text: root.detail ? root.detail.errors.join('\n\n') : ''; color: Theme.dangerText }
        Quiet { Layout.fillWidth: true; text: root.detail ? root.detail.warnings.join('\n\n') : '' }
        Item { Layout.fillHeight: true }
    }
}
