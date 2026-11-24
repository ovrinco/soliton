import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Workplane.js" as Workplane
import "CalendarMath.js" as Cal

Item {
    id: root
    property real completionClock: Date.now()
    Timer { interval: 250; repeat: true; running: root.visible && !!(Backend.prefs.lastTaskCompletion || {}).undoUntil && (Backend.prefs.lastTaskCompletion || {}).undoUntil > root.completionClock; onTriggered: root.completionClock = Date.now() }
    objectName: 'record-sheet'
    property var d: Backend.activeDraft || {}
    readonly property bool isDocument: ['note','learning','strategy'].indexOf(d.kind) >= 0
    readonly property bool isTask: d.kind === 'task'
    readonly property bool markable: !!d.id && ['task','note','learning','strategy','goal','category'].indexOf(d.kind) >= 0
    readonly property bool markedPrivate: markable && !Backend.privateMode && Backend.isMarkedPrivate(d.id)
    readonly property bool occurrenceLocked: !!(d.occurrence && d.occurrenceOnly)
    readonly property string identity: (d.id || 'new') + ':' + (d.kind || '')
    property bool preview: false
    property bool descriptionRead: true
    property string retainedIdentity: ''
    property bool ownSave: false
    property bool closeAfterSave: false
    property var pendingCompletion: null
    property string closeIdentity: ''
    property string closeDraftKey: ''
    property bool closeWasNew: false
    onIdentityChanged: {
        // The owner callback clears saving before publishing the saved identity.
        // An identity change while saving is therefore navigation, not settlement.
        if (ownSave && Backend.saving) ownSave = false;
        if (!ownSave) {
            descriptionRead = !!d.id;
            retainedIdentity = identity;
            preview = isDocument && !!d.id && !d.dirty;
            scheduleDetail.expanded = false; relations.expanded = false; connected.expanded = d.kind === 'goal';
        }
        if (closeAfterSave && (Backend.saving || d.dirty)) closeAfterSave = false;
        saveReview.close(); discardReview.close(); more.close(); documentProperties.close();
    }
    Component.onCompleted: { retainedIdentity = identity; preview = isDocument && !!d.id && !d.dirty; }
    function settleOwnSave() {
        if (!ownSave) return;
        if (retainedIdentity !== identity && !!d.id) {
            var positions = Object.assign({}, Backend.prefs.scrollPositions || {});
            positions['sheet:' + identity] = {x:detailScroll.contentX,y:detailScroll.contentY};
            positions['document:' + identity] = {x:documentReader.scroll.contentX,y:documentReader.scroll.contentY};
            Backend.setting('scrollPositions',positions);
            retainedIdentity = identity;
        }
        ownSave = false;
    }
    // The menu is placed at its button when it opens, where the button is then:
    // a binding read the button before the pane had moved into place.
    function openMore() {
        var p = moreButton.mapToItem(more.parent, 0, 0);
        var placement = Workplane.popupPosition({x:p.x,y:p.y,width:moreButton.width,height:moreButton.height},
            {width:more.width,height:more.height},{width:more.parent.width,height:more.parent.height});
        more.x = placement.x; more.y = placement.y;
        more.open();
    }
    function requestClose() {
        if (Backend.saving) return;
        if (d.dirty) saveReview.open();
        else Backend.closeDetails();
    }
    function saveCurrent() {
        if (Backend.saving) return;
        var stored = root.isTask && d.id ? Backend.get(d.id) : null;
        pendingCompletion = stored && stored.status !== 'completed' && d.status === 'completed' && d.recurrence === 'none'
            ? {id:d.id, previous:stored.status} : null;
        ownSave = true;
        Backend.saveDraft();
    }
    function saveAndClose() {
        closeIdentity = identity; closeDraftKey = Backend.activeKey; closeWasNew = !d.id; closeAfterSave = true;
        saveReview.close(); saveCurrent();
    }
    Connections {
        target: Backend
        function onSavingChanged() {
            if (!Backend.saving && root.ownSave) Qt.callLater(function() { root.settleOwnSave(); });
            if (!Backend.saving && root.pendingCompletion) Qt.callLater(function() {
                var pending = root.pendingCompletion, current = pending ? Backend.get(pending.id) : null;
                if (current && current.status === 'completed') {
                    var now = Date.now();
                    Backend.setting('lastTaskCompletion', {entries:[{id:pending.id,previous:pending.previous,afterHash:current.hash,occurrence:'',occurrence_date:''}],graceUntil:now+4000,undoUntil:now+30000});
                }
                root.pendingCompletion = null;
            });
            if (!Backend.saving && root.closeAfterSave) Qt.callLater(function() {
                if (root.closeAfterSave && (root.identity === root.closeIdentity || (root.closeWasNew && !Backend.drafts[root.closeDraftKey])) && !!root.d.id && !root.d.dirty) Backend.closeDetails();
                root.closeAfterSave = false;
            });
        }
    }
    ColumnLayout {
        anchors.fill: parent
        spacing: 0
        Item {
            Layout.fillWidth: true; Layout.preferredHeight: 52
            RowLayout {
                anchors.fill: parent; anchors.leftMargin: 24; anchors.rightMargin: 16; spacing: 4
                RowLayout {
                    visible: root.isDocument
                    STabButton { text: 'Read'; active: root.preview; onClicked: root.preview = true }
                    STabButton { text: 'Edit'; active: !root.preview; onClicked: root.preview = false }
                }
                Item { Layout.fillWidth: true }
                PrivateMarker { visible: root.markedPrivate }
                SButton { text: 'Save'; primary: true; visible: root.d.kind === 'goal' && !!root.d.dirty; enabled: !Backend.saving && !!String(root.d.title || '').trim(); onClicked: root.saveCurrent() }
                SButton { id: moreButton; iconName: 'more'; accessibleLabel: 'More actions'; quiet: true; onClicked: root.openMore() }
                SButton { iconName: Backend.detailFull ? 'dock' : 'expand'; accessibleLabel: Backend.detailFull ? 'Dock details' : 'Expand details'; quiet: true; onClicked: Backend.detailFull = !Backend.detailFull }
                SButton { iconName: 'close'; accessibleLabel: 'Close details'; quiet: true; enabled: !Backend.saving; onClicked: root.requestClose() }
            }
        }
        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.line }
        // Documents read as documents. Object metadata is available from More,
        // not spread across four competing tabs above the text.
        ColumnLayout {
            Layout.fillWidth: true; Layout.fillHeight: true
            visible: root.isDocument
            spacing: 0
            Item {
                Layout.fillWidth: true
                Layout.preferredHeight: root.preview ? documentTitle.implicitHeight + 40 : 88
                LabelText {
                    id: documentTitle
                    visible: root.preview
                    x: Math.max(24,(parent.width-width)/2); y: 24; width: Math.max(0,Math.min(Theme.readerMeasure,parent.width-48))
                    text: root.d.title || 'Untitled'
                    font.pixelSize: 27; font.weight: Font.DemiBold; font.letterSpacing: -0.6
                    wrapMode: Text.Wrap; elide: Text.ElideNone
                }
                Field {
                    visible: !root.preview
                    x: Math.max(24,(parent.width-width)/2); y: 24
                    width: Math.max(0,Math.min(Theme.readerMeasure,parent.width-48))
                    frameless: true
                    text: root.d.title || ''; placeholderText: 'Untitled'
                    font.pixelSize: 25; font.weight: Font.DemiBold
                    onTextEdited: Backend.edit('title', text)
                    Accessible.name: 'Document title'
                }
            }
            DocumentReader {
                id: documentReader
                Layout.fillWidth: true; Layout.fillHeight: true
                title: root.d.title || 'Document'
                positionKey: 'document:' + root.retainedIdentity
                text: root.d.body || ''
                editable: !root.preview
                textFormat: root.preview ? TextEdit.MarkdownText : TextEdit.PlainText
                onEdited: value => Backend.edit('body', value)
            }
        }
        Flickable {
            id: detailScroll
            objectName: 'record-sheet-scroll'
            Layout.fillWidth: true; Layout.fillHeight: true
            visible: !root.isDocument
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            flickableDirection: Flickable.VerticalFlick
            contentWidth: width
            contentHeight: form.implicitHeight + 44
            ScrollBar.vertical: SScrollBar {}
            RetainedScroll { flickable: detailScroll; positionKey: 'sheet:' + root.retainedIdentity }
            ColumnLayout {
                id: form
                x: Math.max(24,(detailScroll.width-width)/2); y: 24
                width: Math.max(0,Math.min(Theme.readerMeasure,detailScroll.width-48))
                spacing: 16
                Choice {
                    visible: !!root.d.occurrence
                    Layout.fillWidth: true
                    model: ['This occurrence','Whole series']
                    currentIndex: root.d.occurrenceOnly ? 0 : 1
                    onActivated: { if (index === 1) Backend.select(root.d.id); else Backend.edit('occurrenceOnly', true); }
                    Accessible.name: 'Edit scope for recurring task'
                }
                STextArea {
                    id: taskTitle
                    Layout.fillWidth: true
                    Layout.minimumHeight: 44
                    text: root.d.title || ''
                    placeholderText: root.isTask ? 'What needs doing?' : 'Title'
                    font.pixelSize: 25; font.weight: Font.DemiBold
                    padding: 0
                    wrapMode: TextEdit.Wrap
                    background: Rectangle { color: 'transparent'; border.color: taskTitle.activeFocus ? Theme.accent : 'transparent' }
                    onTextChanged: if (activeFocus && text !== (root.d.title || '')) Backend.edit('title', text)
                    Accessible.name: 'Title'
                }
                // The record's tracker row, when it has one: its status word and purpose.
                SRowLink { Layout.fillWidth: true; rowId: root.d.row_id || '' }
                // Four familiar properties, aligned as two label/value rows.
                GridLayout {
                    Layout.fillWidth: true
                    visible: root.isTask || root.d.kind === 'goal'
                    columns: 2; columnSpacing: 14; rowSpacing: 8
                    LabelText { text: 'Category'; color: Theme.muted; font.pixelSize: 13 }
                    Choice {
                        Layout.fillWidth: true
                        enabled: !(root.d.goal_id && root.isTask) && !root.occurrenceLocked
                        model: [{id:'',title:'Unassigned'}].concat(Backend.categories); textRole: 'title'
                        currentIndex: {
                            var goal = root.isTask && root.d.goal_id ? Backend.get(root.d.goal_id) : null;
                            return Math.max(0, model.findIndex(c => c.id === (goal ? goal.category_id : root.d.category_id)));
                        }
                        onActivated: Backend.edit('category_id', model[index].id)
                        Accessible.name: 'Category'
                    }
                    LabelText { visible: root.isTask; text: 'Goal'; color: Theme.muted; font.pixelSize: 13 }
                    Choice {
                        visible: root.isTask; Layout.fillWidth: true
                        enabled: !root.occurrenceLocked
                        model: [{id:'',title:'No goal'}].concat(Backend.goals); textRole: 'title'
                        currentIndex: Math.max(0, model.findIndex(g => g.id === root.d.goal_id))
                        onActivated: { Backend.edit('goal_id', model[index].id); if (index) Backend.edit('category_id', model[index].category_id); }
                        Accessible.name: 'Goal'
                    }
                    LabelText { text: 'Status'; color: Theme.muted; font.pixelSize: 13 }
                    RowLayout {
                        Layout.fillWidth: true
                        Choice {
                            Layout.fillWidth: true
                            model: root.d.kind === 'goal'
                                ? [{label:'Active', value:'active'}, {label:'Completed', value:'completed'}, {label:'Archived', value:'archived'}]
                                : [{label:'To do', value:'todo'}, {label:'In progress', value:'in_progress'}, {label:'Completed', value:'completed'}]
                            textRole: 'label'
                            valueRole: 'value'
                            currentIndex: Math.max(0, model.findIndex(s => s.value === root.d.status))
                            displayText: model[currentIndex].label
                            onActivated: {
                                var nextStatus = model[index].value;
                                if (root.isTask && !root.occurrenceLocked && root.d.recurrence !== 'none' && nextStatus === 'completed') Cal.completion(Backend, root.d, nextStatus);
                                else if (root.isTask && !root.occurrenceLocked && root.d.recurrence !== 'none' && root.d.status === 'completed' && nextStatus === 'todo') Cal.completion(Backend, root.d, nextStatus);
                                else Backend.edit('status', nextStatus);
                            }
                            Accessible.name: 'Status'
                        }
                        Choice {
                            visible: root.isTask; Layout.preferredWidth: 102; enabled: !root.occurrenceLocked
                            model: ['low','medium','high']; currentIndex: Math.max(0, model.indexOf(root.d.priority))
                            displayText: ['Low','Medium','High'][currentIndex]
                            onActivated: Backend.edit('priority', model[index])
                            Accessible.name: 'Priority'
                        }
                    }
                    LabelText { text: root.isTask ? 'Schedule' : 'Target'; color: Theme.muted; font.pixelSize: 13 }
                    RowLayout {
                        Layout.fillWidth: true
                        SDateField { contextKey: Backend.activeKey;
                            Layout.fillWidth: true
                            dateValue: root.isTask ? (root.d.scheduled_for || '') : (root.d.target_on || '')
                            placeholderText: 'YYYY-MM-DD'
                            accessibleName: root.isTask ? 'Scheduled date' : 'Goal target date'
                            onDateEdited: value => { Backend.edit(root.isTask ? 'scheduled_for' : 'target_on', value); if (root.isTask && !value) Backend.edit('schedule', null); }
                        }
                    }
                }
                RowLayout {
                    visible: root.d.kind === 'category'; Layout.fillWidth: true
                    LabelText { text: 'Color'; color: Theme.muted }
                    Field { Layout.fillWidth: true; text: root.d.color || ''; placeholderText: '#RRGGBB'; onTextEdited: Backend.edit('color',text); Accessible.name: 'Category color' }
                }
                Flow {
                    visible: root.d.kind === 'category'; Layout.fillWidth: true; spacing: 8
                    Repeater {
                        model: ['#d4b77c','#83b6a0','#85a9d2','#bb9fcf','#d99984','#a4b974']
                        AbstractButton {
                            required property string modelData
                            width: 32; height: 32; focusPolicy: Qt.StrongFocus
                            Accessible.name: 'Use color ' + modelData
                            Accessible.selected: root.d.color === modelData
                            onClicked: Backend.edit('color',modelData)
                            background: Rectangle { color: parent.modelData; border.width: parent.visualFocus || root.d.color === parent.modelData ? 2 : 0; border.color: Theme.text }
                        }
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    LabelText { text: 'Description'; color: Theme.muted; Layout.fillWidth: true; font.pixelSize: 13 }
                    STabButton { text: 'Read'; active: root.descriptionRead; onClicked: root.descriptionRead = true }
                    STabButton { text: 'Edit'; active: !root.descriptionRead; enabled: !root.occurrenceLocked; onClicked: root.descriptionRead = false }
                }
                Item {
                    id: descriptionHost
                    Layout.fillWidth: true
                    Layout.minimumHeight: 120
                    Layout.preferredHeight: Math.max(120, description.implicitHeight, descriptionPreview.implicitHeight)
                    // Separate documents: switching format must never serialize
                    // rendered Markdown back into the user's source draft.
                    STextArea {
                        id: description
                        anchors.fill: parent
                        visible: !root.descriptionRead
                        readOnly: root.occurrenceLocked
                        textFormat: TextEdit.PlainText
                        text: root.d.body || ''
                        placeholderText: 'Add a description…'
                        wrapMode: TextEdit.Wrap
                        font.pixelSize: 15; padding: 12
                        onTextChanged: if (activeFocus && !root.descriptionRead && text !== (root.d.body || '')) Backend.edit('body',text)
                        Accessible.name: 'Edit description'
                    }
                    STextArea {
                        id: descriptionPreview
                        anchors.fill: parent
                        visible: root.descriptionRead
                        readOnly: true
                        textFormat: TextEdit.MarkdownText
                        text: root.d.body || ''
                        placeholderText: 'No description'
                        wrapMode: TextEdit.Wrap
                        font.pixelSize: 15; padding: 12
                        background: null
                        onLinkActivated: link => Qt.openUrlExternally(link)
                        Accessible.name: 'Read description'
                    }
                }                SDisclosure {
                    id: scheduleDetail
                    Layout.fillWidth: true
                    visible: root.isTask
                    title: 'Timing & repeat'
                    summary: (root.d.schedule ? (root.d.schedule.time || 'Timed') : '') + (root.d.recurrence && root.d.recurrence !== 'none' ? '  ' + root.d.recurrence : '')
                    GridLayout {
                        Layout.fillWidth: true; columns: 2; columnSpacing: 12; rowSpacing: 8
                        LabelText { text: 'Time'; color: Theme.muted }
                        Choice { Layout.fillWidth: true; enabled: !!root.d.scheduled_for; model: ['All day','Timed block']; currentIndex: root.d.schedule ? 1 : 0; onActivated: { if (index === 0) Backend.edit('schedule',null); else Backend.editSchedule('time','09:00'); } }
                        LabelText { visible: !!root.d.schedule; text: 'Start / min'; color: Theme.muted }
                        RowLayout {
                            visible: !!root.d.schedule; Layout.fillWidth: true
                            Field { Layout.fillWidth: true; text: (root.d.schedule || {}).time || ''; placeholderText: 'HH:MM'; onTextEdited: Backend.editSchedule('time',text); Accessible.name: 'Start time' }
                            Field { Layout.preferredWidth: 70; text: String((root.d.schedule || {}).duration_minutes || 60); inputMethodHints: Qt.ImhDigitsOnly; onTextEdited: Backend.editSchedule('duration_minutes',Number(text)); Accessible.name: 'Duration in minutes' }
                        }
                        LabelText { text: 'Due'; color: Theme.muted }
                        SDateField { contextKey: Backend.activeKey; Layout.fillWidth: true; enabled: !root.occurrenceLocked; dateValue: root.d.due_on || ''; placeholderText: 'YYYY-MM-DD'; accessibleName: 'Due date'; onDateEdited: value => Backend.edit('due_on', value) }
                        LabelText { text: 'Repeat'; color: Theme.muted }
                        Choice {
                            Layout.fillWidth: true; enabled: !root.occurrenceLocked
                            model: ['none','daily','weekdays','weekly','biweekly','monthly','yearly']; currentIndex: Math.max(0,model.indexOf(root.d.recurrence))
                            displayText: ['Does not repeat','Daily','Weekdays','Weekly','Every two weeks','Monthly','Yearly'][currentIndex]
                            onActivated: { Backend.edit('recurrence',model[index]); if (!root.d.repeat_start) Backend.edit('repeat_start', root.d.scheduled_for || Qt.formatDate(new Date(),'yyyy-MM-dd')); }
                        }
                        LabelText { visible: !!root.d.recurrence && root.d.recurrence !== 'none'; text: 'Interval / end'; color: Theme.muted }
                        RowLayout {
                            visible: !!root.d.recurrence && root.d.recurrence !== 'none'; enabled: !root.occurrenceLocked; Layout.fillWidth: true
                            Field { Layout.preferredWidth: 54; text: String(root.d.interval || 1); onTextEdited: Backend.edit('interval',Number(text)); Accessible.name: 'Repeat interval' }
                            SDateField { contextKey: Backend.activeKey; Layout.fillWidth: true; dateValue: root.d.repeat_until || ''; placeholderText: 'No end date'; accessibleName: 'Repeat until'; onDateEdited: value => Backend.edit('repeat_until', value) }
                        }
                    }
                    SDisclosure {
                        Layout.fillWidth: true; visible: !!root.d.schedule
                        title: 'Time zone'
                        summary: (root.d.schedule || {}).timezone || Backend.prefs.timezone || ''
                        Field { Layout.fillWidth: true; text: (root.d.schedule || {}).timezone || Backend.prefs.timezone || ''; placeholderText: 'Time zone'; onTextEdited: Backend.editSchedule('timezone',text); Accessible.name: 'Time zone' }
                        Choice {
                            Layout.fillWidth: true
                            model: ['Ask for repeated hour','Earlier repeated hour','Later repeated hour']
                            currentIndex: !root.d.schedule || !root.d.schedule.fold ? 0 : root.d.schedule.fold === 'later' ? 2 : 1
                            onActivated: Backend.editSchedule('fold',['','earlier','later'][index])
                            Accessible.name: 'Daylight saving repeated hour'
                        }
                    }
                }
                SDisclosure {
                    id: connected
                    Layout.fillWidth: true
                    title: root.d.kind === 'goal' ? 'Tasks & notes' : 'Related notes'
                    summary: String(relatedRepeater.count)
                    RowLayout {
                        Layout.fillWidth: true
                        SButton { text: 'Linked note'; iconName: 'plus'; quiet: true; enabled: !!root.d.id && !root.d.dirty; onClicked: { var id = root.d.id; Backend.create('note'); Backend.edit('links',[id]); } }
                        SButton { visible: root.d.kind === 'goal'; enabled: !!root.d.id; text: 'Task'; iconName: 'plus'; quiet: true; onClicked: Backend.create('task',root.d.id,root.d.category_id,Backend.today) }
                        SCheckBox { visible: root.d.kind === 'goal'; text: 'Show completed'; checked: Backend.showCompleted; onToggled: { Backend.showCompleted = checked; Backend.remember(); } }
                        SButton { visible: root.d.kind === 'goal' && Cal.canUndo(Backend, root.completionClock); text: 'Undo'; quiet: true; onClicked: Cal.undoCompletion(Backend) }
                    }
                    Repeater {
                        id: relatedRepeater
                        model: Backend.records.filter(r => !r.hidden && (r.kind !== 'task' || Cal.showTask(Backend, r, root.completionClock)) && ((r.links || []).indexOf(root.d.id) >= 0 || !!root.d.id && r.goal_id === root.d.id))
                        Item {
                            required property var modelData
                            Layout.fillWidth: true
                            implicitHeight: modelData.kind === 'task' ? 62 : 46
                            TaskCard { anchors.fill: parent; visible: parent.modelData.kind === 'task'; record: parent.modelData; compact: true; draggable: false }
                            MenuAction { anchors.fill: parent; visible: parent.modelData.kind !== 'task'; text: parent.modelData.title; onClicked: Backend.select(parent.modelData.id) }
                        }
                    }
                }
                SDisclosure {
                    id: relations
                    Layout.fillWidth: true
                    title: 'Files, links & tags'
                    summary: String((root.d.files || []).length + (root.d.links || []).length)
                    RecordRelations { Layout.fillWidth: true; record: root.d }
                }
            }
        }
        Rectangle { visible: (!root.isDocument || !root.preview) && (root.d.dirty || Backend.saving) && root.d.kind !== 'goal'; Layout.fillWidth: true; height: 1; color: Theme.line }
        Item {
            visible: (!root.isDocument || !root.preview) && (root.d.dirty || Backend.saving) && root.d.kind !== 'goal'
            Layout.fillWidth: true; Layout.preferredHeight: 60
            RowLayout {
                anchors.fill: parent; anchors.leftMargin: 24; anchors.rightMargin: 24; spacing: 8
                LabelText { Layout.fillWidth: true; text: Backend.saving ? 'Saving…' : root.d.dirty ? 'Unsaved changes' : ''; font.pixelSize: 12; color: Theme.muted }
                SButton { text: 'Discard'; quiet: true; enabled: !!root.d.dirty && !Backend.saving; onClicked: discardReview.open() }
                SButton { text: 'Save'; primary: true; enabled: !!String(root.d.title || '').trim() && !!root.d.dirty && !Backend.saving; onClicked: root.saveCurrent() }
            }
        }
    }
    SPopup {
        id: more
        parent: Overlay.overlay
        width: 238; padding: 8
        focus: true
        contentItem: ColumnLayout {
            spacing: 0
            MenuAction { visible: root.isDocument; text: 'Files, links & tags'; iconName: 'source'; onClicked: { more.close(); documentProperties.open(); } }
            MenuAction { text: 'Version history'; iconName: 'clock'; enabled: !!root.d.id && !root.d.dirty; onClicked: { more.close(); versions.showFor(root.d.id); } }
            MenuAction { text: 'Duplicate'; iconName: 'copy'; enabled: !!root.d.id && !root.occurrenceLocked; onClicked: { more.close(); Backend.send('duplicate',{id:root.d.id},(value,ok) => { if (ok) Backend.select(value.id); }); } }
            MenuAction { text: root.d.archived ? 'Restore from archive' : 'Archive'; enabled: !!root.d.id && !root.occurrenceLocked; onClicked: { more.close(); Backend.edit('archived',!root.d.archived); root.saveCurrent(); } }
            MenuAction { visible: root.markable; text: root.markedPrivate ? 'Unmark private' : 'Mark private'; iconName: 'private'; onClicked: { var id = root.d.id, on = !root.markedPrivate; more.close(); Backend.setMarkedPrivate(id, on); } }
        }
    }
    SDialog {
        id: documentProperties
        title: 'Files, links & tags'
        parent: Overlay.overlay; anchors.centerIn: parent
        width: Math.min(560,parent.width-32); height: Math.min(600,parent.height-32)
        modal: true; focus: true; standardButtons: Dialog.Close
        contentItem: ScrollView { clip: true; contentWidth: availableWidth; RecordRelations { width: parent.width; record: root.d } }
    }
    RecordHistory { id: versions }
    SDialog {
        id: discardReview
        title: 'Discard changes?'
        parent: Overlay.overlay; anchors.centerIn: parent
        width: Math.min(400,parent.width-32); modal: true; focus: true
        contentItem: ColumnLayout {
            spacing: 16
            LabelText { Layout.fillWidth: true; text: 'Your saved version will stay unchanged.'; wrapMode: Text.Wrap }
            RowLayout {
                Layout.fillWidth: true; Item { Layout.fillWidth: true }
                SButton { text: 'Cancel'; onClicked: discardReview.close() }
                SButton { text: 'Discard'; destructive: true; onClicked: { discardReview.close(); Backend.reloadDraft(); } }
            }
        }
    }
    SDialog {
        id: saveReview
        title: 'Save changes?'
        parent: Overlay.overlay; anchors.centerIn: parent
        width: Math.min(460,parent.width-32); modal: true; focus: true
        contentItem: ColumnLayout {
            spacing: 16
            LabelText { Layout.fillWidth: true; text: root.d.title || 'Untitled'; wrapMode: Text.Wrap }
            RowLayout {
                Layout.fillWidth: true
                SButton { text: 'Cancel'; onClicked: saveReview.close() }
                Item { Layout.fillWidth: true }
                SButton { text: 'Discard'; onClicked: { saveReview.close(); Backend.reloadDraft(); Backend.closeDetails(); } }
                SButton { text: 'Save'; primary: true; enabled: !!String(root.d.title || '').trim(); onClicked: root.saveAndClose() }
            }
        }
    }
}
