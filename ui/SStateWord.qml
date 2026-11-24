import QtQuick

// A state is one word in its tone's colour, never a phrase.
LabelText {
    id: stateWord
    property string word: ''
    // neutral, accent, success or danger
    property string tone: 'neutral'
    text: stateWord.word.split(/\s+/)[0] || ''
    font.pixelSize: Theme.fontSizeSmall
    font.weight: Font.Medium
    color: tone === 'accent' ? Theme.accentText
        : tone === 'success' ? Theme.successText
        : tone === 'danger' ? Theme.dangerText
        : Theme.muted
    Accessible.role: Accessible.StaticText
    Accessible.name: text
}
