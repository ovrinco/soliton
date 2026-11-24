import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import ".."
import "../TranscriptDocument.js" as TranscriptDocument
import "TranscriptTiming.js" as TranscriptTiming
import "../Workplane.js" as Workplane

// The voice-note transcript, read, heard and edited in place. A click places
// the caret in the text and, while the text is unchanged and audio is
// available, also seeks to that passage. Correcting means unsaved changes.
// Marks are drawn behind the text, never in it.
Item {
    id: root
    property string text: ''
    property var document: ({text: '', spans: []})
    property var placed: ({spans: [], anyEstimated: false})
    property bool timingAvailable: false
    property bool audioAvailable: false
    property bool marksShown: false
    property bool playing: false
    property int positionMs: 0
    property bool editable: false
    property string title: 'Transcript'
    property string positionKey: ''
    property bool searching: false
    property string findStatus: ''
    property bool following: true
    property bool scrollingForPlayback: false
    property alias editor: editor
    property alias scroll: viewport
    property alias actions: contextualActions.data
    signal edited(string value)
    signal passageChanged(int start, int end)
    signal seekRequested(int milliseconds)

    readonly property bool changed: root.text !== root.document.text
    readonly property var currentSpan: root.marksShown && !root.changed ? TranscriptDocument.atTime(root.document, root.positionMs) : null
    readonly property var currentWords: root.currentSpan && root.timingAvailable ? (root.placed.spans[root.document.spans.indexOf(root.currentSpan)] || []) : []
    readonly property var currentWord: TranscriptTiming.wordAt(root.currentWords, root.positionMs)

    function restore() { retained.restore(); }
    function follow() { root.following = true; Qt.callLater(root.keepRunVisible); }
    onEditableChanged: if (!editable) replaceDialog.close()
    onCurrentSpanChanged: if (playing) Qt.callLater(keepRunVisible)
    onPlayingChanged: if (playing && following) Qt.callLater(keepRunVisible)

    // Rectangles in editor coordinates covering [start, end) line by line.
    function rangeRects(start, end, revision) {
        if (start === undefined || end <= start || editor.width <= 0) return [];
        var a = editor.positionToRectangle(start), b = editor.positionToRectangle(end);
        if (Math.abs(a.y - b.y) < 1) return [{x: a.x, y: a.y, width: Math.max(1, b.x - a.x), height: a.height}];
        var rects = [{x: a.x, y: a.y, width: Math.max(1, editor.width - a.x), height: a.height}];
        if (b.y - (a.y + a.height) > 1) rects.push({x: 0, y: a.y + a.height, width: editor.width, height: b.y - a.y - a.height});
        if (b.x > 0) rects.push({x: 0, y: b.y, width: b.x, height: b.height});
        return rects;
    }
    function keepRunVisible() {
        if (!root.following || !root.currentSpan) return;
        var rects = rangeRects(root.currentSpan.start, root.currentSpan.end, editor.contentHeight);
        if (!rects.length) return;
        var line = rects[0].height;
        var top = editor.y + rects[0].y - line, last = rects[rects.length - 1];
        var bottom = editor.y + last.y + last.height + line;
        var target = viewport.contentY;
        if (top < viewport.contentY) target = top;
        else if (bottom > viewport.contentY + viewport.height) target = Math.min(top, bottom - viewport.height);
        target = Math.max(0, Math.min(target, viewport.contentHeight - viewport.height));
        if (target === viewport.contentY) return;
        root.scrollingForPlayback = true;
        viewport.contentY = target;
        root.scrollingForPlayback = false;
    }
    function seekAt(x, y, margin) {
        if (root.changed || !root.audioAvailable || !root.document.spans.length) return;
        var position = editor.positionAt(Math.max(0, Math.min(editor.width - 1, x)), y);
        var span = TranscriptDocument.spanAt(root.document, position);
        if (!span) {
            // A paragraph gap belongs to the passage that follows it.
            span = root.document.spans.find(function(item) { return item.start >= position; }) || null;
        }
        if (!span) return;
        var index = root.document.spans.indexOf(span);
        var word = !margin && root.timingAvailable && !root.changed
            ? TranscriptTiming.wordAtOffset(root.placed.spans[index] || [], position) : null;
        root.following = true;
        root.seekRequested(word ? word.startMs : span.startMs);
    }

    function showFind() { searching = true; findField.forceActiveFocus(); findField.selectAll(); }
    function findNext(backwards) {
        var haystack = editor.getText(0, editor.length), needle = findField.text;
        if (!needle) { findStatus = ''; return; }
        var matches = Workplane.literalMatches(haystack, needle);
        var found = null, start = backwards ? editor.selectionStart : editor.selectionEnd;
        if (backwards) {
            for (var i = matches.length - 1; i >= 0; --i) if (matches[i].start < start) { found = matches[i]; break; }
            if (!found && matches.length) found = matches[matches.length - 1];
        } else {
            for (var j = 0; j < matches.length; ++j) if (matches[j].start >= start) { found = matches[j]; break; }
            if (!found && matches.length) found = matches[0];
        }
        findStatus = found ? '' : 'Not found';
        if (found) {
            editor.select(found.start, found.start + found.length);
            var rect = editor.positionToRectangle(found.start);
            root.following = false;
            viewport.contentY = Math.max(0, Math.min(editor.y + rect.y - 32, viewport.contentHeight - viewport.height));
        }
    }
    function showReplace() {
        if (!root.editable) return;
        replaceQuery.text = findField.text; replaceFeedback.text = '';
        replaceDialog.open(); replaceQuery.forceActiveFocus();
    }
    function replaceText(all) {
        if (!root.editable) return;
        var before = editor.text, matches = Workplane.literalMatches(before, replaceQuery.text);
        if (!matches.length) { replaceFeedback.text = 'No matches'; return; }
        if (!all) matches = [matches.find(function(m) { return m.start >= editor.selectionStart; }) || matches[0]];
        root.edited(Workplane.replaceMatches(before, matches, replaceValue.text));
        replaceFeedback.text = matches.length + (matches.length === 1 ? ' replacement' : ' replacements');
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
                SButton { text: 'Replace'; visible: root.editable && (editor.activeFocus || root.changed); quiet: true; onClicked: root.showReplace() }
                SButton { text: ''; iconName: 'search'; quiet: true; accessibleLabel: 'Find in transcript'; enabled: editor.length > 0; onClicked: root.showFind() }
            }
            RowLayout {
                anchors.fill: parent
                visible: root.searching
                SSearchField { id: findField; Layout.fillWidth: true; scopeName: 'this transcript'; Keys.onEscapePressed: root.searching = false
                    onAccepted: root.findNext(false); onTextEdited: root.findStatus = ''; Accessible.name: 'Find in transcript' }
                LabelText { text: root.findStatus; color: Theme.muted; font.pixelSize: 12; visible: root.width >= 520; Layout.preferredWidth: 68 }
                SButton { text: ''; iconName: 'left'; accessibleLabel: 'Previous match'; onClicked: root.findNext(true) }
                SButton { text: ''; iconName: 'right'; accessibleLabel: 'Next match'; onClicked: root.findNext(false) }
                SButton { text: ''; iconName: 'close'; quiet: true; accessibleLabel: 'Close find'; onClicked: root.searching = false }
            }
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line }
        }
        Flickable {
            id: viewport
            objectName: 'transcript-reader-scroll'
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            flickableDirection: Flickable.VerticalFlick
            contentWidth: width
            contentHeight: editor.height + 40
            ScrollBar.vertical: SScrollBar { }
            // A scroll by hand stops following until a word is clicked or Play is pressed.
            onContentYChanged: if (!root.scrollingForPlayback && root.playing && (moving || dragging || verticalScrollBarActive())) root.following = false
            function verticalScrollBarActive() { return ScrollBar.vertical && ScrollBar.vertical.pressed; }
            WheelHandler { onWheel: root.following = false }
            RetainedScroll {
                id: retained
                flickable: viewport
                positionKey: root.positionKey
                active: root.visible && root.text.length > 0 && root.positionKey.length > 0
            }
            MouseArea {
                // The margins of the page: a click seeks to the passage beside it.
                width: viewport.width; height: Math.max(viewport.height, viewport.contentHeight)
                enabled: !root.changed && root.audioAvailable
                onClicked: mouse => root.seekAt(mouse.x - editor.x, mouse.y - editor.y, true)
            }
            Item {
                x: editor.x; y: editor.y; width: editor.width; height: editor.height
                visible: !!root.currentSpan
                Repeater {
                    model: root.currentSpan ? root.rangeRects(root.currentSpan.start, root.currentSpan.end, editor.contentHeight + editor.width) : []
                    delegate: Rectangle {
                        required property var modelData
                        x: modelData.x - 2; y: modelData.y; width: modelData.width + 4; height: modelData.height
                        radius: Theme.cornerRadius
                        color: Theme.wash
                    }
                }
            }
            TextArea {
                id: editor
                objectName: 'transcript-reader-text'
                x: Math.max(24, (viewport.width - width) / 2)
                y: 20
                width: Math.max(0, Math.min(Theme.readerMeasure, viewport.width - 48))
                height: Math.max(implicitHeight, viewport.height - 40)
                padding: 0
                text: root.text
                textFormat: TextEdit.PlainText
                readOnly: !root.editable
                selectByMouse: true
                selectByKeyboard: true
                persistentSelection: true
                wrapMode: TextEdit.Wrap
                font.family: Theme.font
                font.pixelSize: Theme.readerSize
                color: Theme.text
                selectionColor: Theme.accent
                selectedTextColor: Theme.accentForeground
                background: null
                cursorVisible: root.editable && activeFocus
                Accessible.name: root.title
                onTextChanged: if (activeFocus && root.editable && text !== root.text) root.edited(text)
                onSelectionStartChanged: root.passageChanged(selectionStart, selectionEnd)
                onSelectionEndChanged: root.passageChanged(selectionStart, selectionEnd)
                onCursorPositionChanged: if (selectionStart === selectionEnd) root.passageChanged(cursorPosition, cursorPosition)
                Keys.onPressed: event => {
                    if ((event.modifiers & Qt.ControlModifier) && event.key === Qt.Key_H && root.editable) { root.showReplace(); event.accepted = true; }
                    else if ((event.modifiers & Qt.ControlModifier) && event.key === Qt.Key_F) { root.showFind(); event.accepted = true; }
                    else if (event.key === Qt.Key_Escape && root.searching) { root.searching = false; event.accepted = true; }
                }
                TapHandler {
                    // A tap places the caret and seeks; a drag still selects text.
                    enabled: !root.changed && root.audioAvailable
                    gesturePolicy: TapHandler.DragThreshold
                    onTapped: eventPoint => root.seekAt(eventPoint.position.x, eventPoint.position.y, false)
                }
                HoverHandler { cursorShape: root.editable ? Qt.IBeamCursor : (root.audioAvailable ? Qt.PointingHandCursor : Qt.ArrowCursor) }
                Item {
                    // The current word: 2 px accent when measured, 1 px dotted muted when estimated.
                    readonly property var rect: root.currentWord ? editor.positionToRectangle(root.currentWord.start) : null
                    readonly property var rectEnd: root.currentWord ? editor.positionToRectangle(root.currentWord.end) : null
                    visible: !!rect && !!rectEnd && Math.abs(rect.y - rectEnd.y) < 1
                    x: rect ? rect.x : 0
                    y: rect ? rect.y + rect.height - (root.currentWord && root.currentWord.estimated ? 1 : 2) : 0
                    width: rect && rectEnd ? Math.max(1, rectEnd.x - rect.x) : 0
                    height: root.currentWord && root.currentWord.estimated ? 1 : 2
                    clip: true
                    Rectangle { anchors.fill: parent; visible: !!root.currentWord && !root.currentWord.estimated; color: Theme.accent }
                    Row {
                        visible: !!root.currentWord && root.currentWord.estimated
                        spacing: 2
                        Repeater {
                            model: Math.ceil(Math.max(0, parent.parent.width) / 3)
                            delegate: Rectangle { width: 1; height: 1; color: Theme.muted }
                        }
                    }
                }
            }
        }
    }
    Rectangle {
        anchors.fill: parent
        z: 5
        color: 'transparent'
        border.color: root.editable && editor.activeFocus ? Theme.accent : 'transparent'
        border.width: 1
    }
    SDialog {
        id: replaceDialog
        parent: Overlay.overlay; anchors.centerIn: parent
        title: 'Find and replace'; modal: true; focus: true
        width: Math.min(460, parent.width - 32)
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
