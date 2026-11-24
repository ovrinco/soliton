import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Workplane.js" as Workplane

// Native, selectable, continuous text. PlainText is intentional for source
// readers: rich-text layout would change the source/cursor offset contract.
Item {
    id: root
    property string text: ''
    property string title: 'Document'
    property string emptyTitle: 'No text yet'
    property string emptyDetail: ''
    property string positionKey: ''
    property bool editable: false
    property int textFormat: TextEdit.PlainText
    property int fontSize: Theme.readerSize
    property bool searching: false
    property string findStatus: ''
    property string replaceIdentity: ''
    onPositionKeyChanged: replaceDialog.close()
    onEditableChanged: {
        if (!editable) replaceDialog.close();
        else Qt.callLater(function() {
            // Qt rewrites TextArea.text while switching MarkdownText to
            // PlainText. The owner value is the original editable source.
            if (root.editable) editor.text = Qt.binding(function() { return root.text; });
        });
    }
    property alias editor: editor
    property alias scroll: viewport
    property alias actions: contextualActions.data
    signal edited(string value)
    signal passageChanged(int start, int end)
    function restore() { retained.restore(); }
    function showFind() {
        searching = true;
        findField.forceActiveFocus(); findField.selectAll();
    }
    function findNext(backwards) {
        var haystack = editor.getText(0, editor.length);
        var needle = findField.text;
        if (!needle) { findStatus = ''; return; }
        // Use match indices in the ORIGINAL string. Lowercasing the whole
        // document can expand Unicode characters and corrupt source offsets.
        var escaped = needle.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
        var pattern = new RegExp(escaped, 'gi'), matches = [], match;
        while ((match = pattern.exec(haystack)) !== null)
            matches.push({ start: match.index, length: match[0].length });
        var found = null, start = backwards ? editor.selectionStart : editor.selectionEnd;
        if (backwards) {
            for (var i = matches.length-1; i >= 0; --i)
                if (matches[i].start < start) { found = matches[i]; break; }
            if (!found && matches.length) found = matches[matches.length-1];
        } else {
            for (var j = 0; j < matches.length; ++j)
                if (matches[j].start >= start) { found = matches[j]; break; }
            if (!found && matches.length) found = matches[0];
        }
        findStatus = !found ? 'Not found' : '';
        if (found) {
            editor.select(found.start, found.start + found.length);
            var rect = editor.positionToRectangle(found.start);
            viewport.contentY = Math.max(0, Math.min(editor.y + rect.y - 32, viewport.contentHeight - viewport.height));
        }
    }

    function showReplace() {
        if (!editable || textFormat !== TextEdit.PlainText) return;
        replaceIdentity = positionKey;
        replaceQuery.text = findField.text;
        replaceFeedback.text = '';
        replaceDialog.open();
        replaceQuery.forceActiveFocus();
    }
    function replaceText(all) {
        if (!editable || textFormat !== TextEdit.PlainText || replaceIdentity !== positionKey) return;
        var before = editor.text;
        var matches = Workplane.literalMatches(before, replaceQuery.text);
        if (!matches.length) { replaceFeedback.text = 'No matches'; return; }
        if (!all) {
            var next = matches.find(function(m) { return m.start >= editor.selectionStart; }) || matches[0];
            matches = [next];
        }
        var after = Workplane.replaceMatches(before, matches, replaceValue.text);
        root.edited(after); // Draft only. The existing Save/Discard boundary is unchanged.
        replaceFeedback.text = matches.length + (matches.length === 1 ? ' replacement' : ' replacements');
        if (!all) Qt.callLater(function() {
            editor.select(matches[0].start, matches[0].start + replaceValue.text.length);
        });
    }

    function keepEditingCursorVisible() {
        if (!root.editable || !editor.activeFocus || editor.selectionStart !== editor.selectionEnd) return;
        var r = editor.cursorRectangle, top = editor.y + r.y, bottom = top + r.height;
        if (top < viewport.contentY + 12) viewport.contentY = Math.max(0, top - 12);
        else if (bottom > viewport.contentY + viewport.height - 12) viewport.contentY = Math.max(0, Math.min(bottom - viewport.height + 12, viewport.contentHeight - viewport.height));
    }

    function copyText() {
        if (editor.selectionStart !== editor.selectionEnd) { editor.copy(); return; }
        var cursor = editor.cursorPosition, x = viewport.contentX, y = viewport.contentY;
        editor.selectAll(); editor.copy(); editor.deselect(); editor.cursorPosition = cursor;
        viewport.contentX = x; viewport.contentY = y;
    }
    onTextChanged: if (!editor.activeFocus && text.length) Qt.callLater(restore)
    ColumnLayout {
        anchors.fill: parent
        spacing: 0
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.actionHeight
            RowLayout {
                anchors.fill: parent
                visible: !root.searching
                spacing: 6
                RowLayout { id: contextualActions; spacing: 6 }
                Item { Layout.fillWidth: true }
                SButton { text: 'Replace'; visible: root.editable; quiet: true; onClicked: root.showReplace() }
                SButton { text: root.width >= 480 ? 'Find' : ''; iconName: 'search'; quiet: true; accessibleLabel: 'Find in document'; onClicked: root.showFind() }
                SButton { text: ''; iconName: 'copy'; quiet: true; accessibleLabel: editor.selectedText ? 'Copy selected text' : 'Copy document'; enabled: editor.length > 0; onClicked: root.copyText() }
            }
            RowLayout {
                anchors.fill: parent
                visible: root.searching
                SSearchField { id: findField; Layout.fillWidth: true; scopeName: 'this document'; Keys.onEscapePressed: { root.searching = false; editor.forceActiveFocus(); }
                onAccepted: root.findNext(false); onTextEdited: root.findStatus = ''; Accessible.name: 'Find in document' }
                LabelText { text: root.findStatus; color: Theme.muted; font.pixelSize: 12; visible: root.width >= 520; Layout.preferredWidth: 68 }
                SButton { text: ''; iconName: 'left'; accessibleLabel: 'Previous match'; onClicked: root.findNext(true) }
                SButton { text: ''; iconName: 'right'; accessibleLabel: 'Next match'; onClicked: root.findNext(false) }
                SButton { text: ''; iconName: 'close'; quiet: true; accessibleLabel: 'Close find'; onClicked: { root.searching = false; editor.forceActiveFocus(); } }
            }
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line }
        }
        Flickable {
            id: viewport
            objectName: 'document-reader-scroll'
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            flickableDirection: Flickable.VerticalFlick
            contentWidth: width
            contentHeight: editor.height + 40
            ScrollBar.vertical: SScrollBar { }
            RetainedScroll {
                id: retained
                flickable: viewport
                positionKey: root.positionKey
                active: root.visible && root.text.length > 0 && root.positionKey.length > 0
            }
            StateView {
                anchors.fill: parent
                visible: !root.editable && !root.text.length
                title: root.emptyTitle
                detail: root.emptyDetail
            }
            TextArea {
                id: editor
                objectName: 'document-reader-text'
                x: Math.max(24, (viewport.width - width) / 2)
                y: 20
                width: Math.max(0, Math.min(Theme.readerMeasure, viewport.width - 48))
                height: Math.max(implicitHeight, viewport.height - 40)
                padding: 0
                text: root.text
                textFormat: root.textFormat
                readOnly: !root.editable
                selectByMouse: true
                selectByKeyboard: true
                persistentSelection: true
                wrapMode: TextEdit.Wrap
                font.family: Theme.font
                font.pixelSize: root.fontSize
                color: Theme.text
                selectionColor: Theme.accent
                selectedTextColor: Theme.accentForeground
                placeholderText: root.editable ? 'Write here…' : ''
                placeholderTextColor: Theme.muted
                background: null
                Accessible.name: root.title
                onLinkActivated: link => { if (!root.editable) Qt.openUrlExternally(link); }
                onTextChanged: if (activeFocus && root.editable && text !== root.text) root.edited(text)
                onSelectionStartChanged: root.passageChanged(selectionStart, selectionEnd)
                onSelectionEndChanged: root.passageChanged(selectionStart, selectionEnd)
                onCursorPositionChanged: if (selectionStart === selectionEnd) root.passageChanged(cursorPosition, cursorPosition)
                Keys.onReleased: event => {
                    if (!(event.modifiers & Qt.ControlModifier) && root.editable) root.keepEditingCursorVisible();
                }
                Keys.onPressed: event => {
                    if ((event.modifiers & Qt.ControlModifier) && event.key === Qt.Key_H && root.editable) { root.showReplace(); event.accepted = true; }
                    else if ((event.modifiers & Qt.ControlModifier) && event.key === Qt.Key_F) { root.showFind(); event.accepted = true; }
                    else if (event.key === Qt.Key_Escape && root.searching) { root.searching = false; event.accepted = true; }
                }
            }
        }
    }
    Rectangle {
        anchors.fill: parent
        z: 5
        color: 'transparent'
        border.color: editor.activeFocus ? Theme.accent : 'transparent'
        border.width: 1
        // Deliberately outside the layout: keyboard focus changes no geometry.
    }
    SDialog {
        id: replaceDialog
        parent: Overlay.overlay; anchors.centerIn: parent
        title: 'Find and replace'; modal: true; focus: true
        width: Math.min(460,parent.width-32)
        contentItem: ColumnLayout {
            spacing: 12
            SSearchField { id: replaceQuery; scopeName: 'this draft'; Layout.fillWidth: true; onAccepted: root.replaceText(false) }
            Field { id: replaceValue; placeholderText: 'Replace with'; Accessible.name: 'Replacement text'; Layout.fillWidth: true }
            LabelText { id: replaceFeedback; Layout.fillWidth: true; color: Theme.muted; text: ''; wrapMode: Text.Wrap }
            RowLayout {
                Layout.fillWidth: true
                SButton { text: 'Close'; quiet: true; onClicked: replaceDialog.requestDismiss() }
                Item { Layout.fillWidth: true }
                SButton { text: 'Replace'; enabled: replaceQuery.text.length > 0; onClicked: root.replaceText(false) }
                SButton { text: 'Replace all'; primary: true; enabled: replaceQuery.text.length > 0; onClicked: root.replaceText(true) }
            }
        }
    }

}
