pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The right pane of Finance: the intake list, a statement's bank, a new bank,
// a receipt's lines against its picture, cash by hand and a week's lines of
// one category. The page holds the state; this pane shows one of them.
Item {
    id: pane
    objectName: 'finance-pane'
    property var view: null
    readonly property string kind: view ? view.paneKind : ''
    readonly property var item: view ? view.paneItem : null

    function close() { view.closePane(); }
    function title() {
        switch (kind) {
        case 'intake': return 'Intake';
        case 'bank': case 'receipt': return item ? item.name : '';
        case 'newBank': return 'New bank';
        case 'cash': return 'Cash';
        case 'lines': return item ? view.word(item.category) + ' · Week of ' + Backend.longDate(item.week) : '';
        }
        return '';
    }
    Keys.onEscapePressed: close()

    component Label: LabelText { font.pixelSize: 12; color: Theme.muted }
    component Labelled: ColumnLayout {
        property alias label: caption.text
        Layout.fillWidth: true; spacing: 6
        Label { id: caption }
    }
    component Footer: RowLayout {
        id: footer
        property bool canSave: true
        property bool busy: false
        signal saved()
        Layout.fillWidth: true
        Layout.margins: 16
        Item { Layout.fillWidth: true }
        SButton { text: 'Cancel'; quiet: true; onClicked: pane.close() }
        SButton { objectName: 'finance-pane-save'; text: 'Save'; primary: true; busy: footer.busy; enabled: footer.canSave; onClicked: footer.saved() }
    }
    component CategoryChoice: Choice {
        property string key: ''
        model: pane.view.categories.map(c => c.word)
        currentIndex: Math.max(0, pane.view.categories.findIndex(c => c.key === key))
        Accessible.name: 'Category'
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0
        Item {
            Layout.fillWidth: true; Layout.preferredHeight: 52
            RowLayout {
                anchors.fill: parent; anchors.leftMargin: 24; anchors.rightMargin: 16; spacing: 4
                LabelText { Layout.fillWidth: true; text: pane.title(); font.pixelSize: 14; font.weight: Font.Medium; elide: Text.ElideRight }
                SButton { objectName: 'finance-pane-close'; iconName: 'close'; accessibleLabel: 'Close'; quiet: true; onClicked: pane.close() }
            }
        }
        Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.line }
        Loader {
            id: body
            Layout.fillWidth: true; Layout.fillHeight: true
            sourceComponent: {
                switch (pane.kind) {
                case 'intake': return intakePane;
                case 'bank': return bankPane;
                case 'newBank': return newBankPane;
                case 'receipt': return receiptPane;
                case 'cash': return cashPane;
                case 'lines': return linesPane;
                }
                return null;
            }
            // A new opening starts the pane afresh.
            Connections {
                target: pane.view
                function onPaneSerialChanged() { body.active = false; body.active = true; }
            }
        }
    }

    // The files in the tool's folder, each by its name, day and one word.
    Component {
        id: intakePane
        ColumnLayout {
            id: f
            spacing: 0
            Component.onCompleted: pane.view.loadIntake()
            Flickable {
                id: intakeScroll
                Layout.fillWidth: true; Layout.fillHeight: true
                clip: true
                contentWidth: width
                contentHeight: intakeBody.implicitHeight + 32
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: SScrollBar { }
                ColumnLayout {
                    id: intakeBody
                    x: 16; y: 16
                    width: intakeScroll.width - 32
                    spacing: 2
                    LabelText { visible: pane.view.intakeLoaded && pane.view.intake.length === 0; Layout.leftMargin: 8; text: 'Empty'; color: Theme.muted }
                    Repeater {
                        model: pane.view.intake
                        delegate: HearthRow {
                            id: file
                            required property var modelData
                            readonly property var document: pane.view.documentOf(modelData)
                            readonly property bool isIn: !!document && document.lines > 0
                            Layout.fillWidth: true
                            objectName: 'intake-' + modelData.name
                            name: modelData.name
                            nameWidth: 170
                            opens: false
                            cells: [{text: Backend.longDate(modelData.day), muted: true, small: true, width: 80}]
                            actions: file.isIn || !modelData.kind ? [] : [{text: 'Import', run: () => pane.view.importFile(modelData)}]
                            LabelText {
                                objectName: 'intake-lines-' + file.modelData.name
                                anchors.right: state.left; anchors.rightMargin: 12
                                anchors.verticalCenter: parent.verticalCenter
                                visible: file.isIn
                                text: file.isIn ? file.document.lines + (file.document.lines === 1 ? ' line' : ' lines') : ''
                                font.pixelSize: 12; color: Theme.muted
                            }
                            SStateWord {
                                id: state
                                objectName: 'intake-state-' + file.modelData.name
                                anchors.right: parent.right; anchors.rightMargin: 40
                                anchors.verticalCenter: parent.verticalCenter
                                width: 48
                                word: file.isIn ? 'in' : 'waiting'
                                tone: file.isIn ? 'success' : 'neutral'
                            }
                        }
                    }
                }
            }
        }
    }

    // A statement asks once which bank wrote it; its mapping is kept.
    Component {
        id: bankPane
        ColumnLayout {
            id: f
            property bool busy: false
            function run() {
                var bank = bankField.text.trim();
                if (!bank || busy) return;
                busy = true;
                pane.view.importStatement(pane.item, bank, () => f.busy = false);
            }
            Component.onCompleted: { bankField.forceActiveFocus(); Backend.loadBanks(); }
            spacing: 0
            ColumnLayout {
                Layout.fillWidth: true; Layout.margins: 24; spacing: 18
                Labelled { label: 'Bank'; Field { id: bankField; objectName: 'import-bank'; Layout.fillWidth: true; Accessible.name: 'Bank'; onAccepted: f.run() } }
                Flow {
                    Layout.fillWidth: true; spacing: 4
                    visible: Backend.financeBanks.length > 0
                    Repeater {
                        model: Backend.financeBanks
                        delegate: SButton { required property var modelData; required property int index; objectName: 'import-bank-pick-' + index; text: modelData.bank; quiet: true; onClicked: bankField.text = modelData.bank }
                    }
                }
            }
            Item { Layout.fillHeight: true }
            Footer { canSave: !!bankField.text.trim(); busy: f.busy; onSaved: f.run() }
        }
    }

    // A bank seen for the first time: its columns, set once.
    Component {
        id: newBankPane
        ColumnLayout {
            id: f
            property bool busy: false
            property var categories: [{bank: '', key: 'food:groceries'}]
            readonly property var formats: ['2026-10-05', '05/10/2026', '10/05/2026']
            readonly property var formatKeys: ['YYYY-MM-DD', 'DD/MM/YYYY', 'MM/DD/YYYY']
            readonly property bool ready: !!bankField.text.trim() && !!column(datePick) && !!column(descriptionPick) && !!column(amountPick) && !!column(currencyPick)
            property var columns: []
            function column(picker) { return picker.currentIndex >= 0 ? f.columns[picker.currentIndex] || '' : ''; }
            function save() {
                if (!ready || busy) return;
                var mapping = {
                    bank: bankField.text.trim(), date: column(datePick), date_format: formatKeys[formatChoice.currentIndex],
                    description: column(descriptionPick), amount: column(amountPick), currency: column(currencyPick),
                    debit_negative: negative.checked, categories: {}
                };
                if (categoryPick.currentIndex > 0) {
                    mapping.category = f.columns[categoryPick.currentIndex - 1];
                    categories.forEach(c => { if (c.bank.trim()) mapping.categories[c.bank.trim()] = c.key; });
                }
                busy = true;
                Backend.financeSaveBank(mapping, ok => {
                    if (!ok) { busy = false; return; }
                    pane.view.importStatement(pane.item, mapping.bank, () => f.busy = false);
                });
            }
            Component.onCompleted: {
                bankField.text = (pane.item || {}).bank || '';
                Backend.send('finance-statement-header', {path: pane.view.intakePath((pane.item || {}).name)}, (r, ok) => {
                    var c = ok ? r.columns || [] : [];
                    f.columns = c;
                    // Each picker starts on the column its name suggests.
                    [[datePick, /date/i], [descriptionPick, /desc|detail|narrat|payee|merchant/i], [amountPick, /amount|value/i], [currencyPick, /curr/i]]
                        .forEach(p => p[0].currentIndex = c.findIndex(n => p[1].test(n)));
                    categoryPick.currentIndex = 1 + c.findIndex(n => /categ|type/i.test(n));
                }, true);
            }
            spacing: 0
            Flickable {
                id: bankScroll
                Layout.fillWidth: true; Layout.fillHeight: true
                clip: true
                contentWidth: width
                contentHeight: bankBody.implicitHeight + 48
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: SScrollBar { }
                ColumnLayout {
                    id: bankBody
                    x: 24; y: 24
                    width: bankScroll.width - 48
                    spacing: 18
                    Labelled { label: 'Bank'; Field { id: bankField; objectName: 'bank-name'; Layout.fillWidth: true; Accessible.name: 'Bank' } }
                    Labelled { label: 'Date'; Choice { id: datePick; objectName: 'bank-date-pick'; Layout.fillWidth: true; model: f.columns; currentIndex: -1; Accessible.name: 'Date column' } }
                    Labelled { label: 'Date format'; Choice { id: formatChoice; objectName: 'bank-format'; Layout.preferredWidth: 200; model: f.formats; Accessible.name: 'Date format' } }
                    Labelled { label: 'Description'; Choice { id: descriptionPick; objectName: 'bank-description-pick'; Layout.fillWidth: true; model: f.columns; currentIndex: -1; Accessible.name: 'Description column' } }
                    Labelled { label: 'Amount'; Choice { id: amountPick; objectName: 'bank-amount-pick'; Layout.fillWidth: true; model: f.columns; currentIndex: -1; Accessible.name: 'Amount column' } }
                    SCheckBox { id: negative; objectName: 'bank-negative'; text: 'Spending negative' }
                    Labelled { label: 'Currency'; Choice { id: currencyPick; objectName: 'bank-currency-pick'; Layout.fillWidth: true; model: f.columns; currentIndex: -1; Accessible.name: 'Currency column' } }
                    Labelled { label: 'Category'; Choice { id: categoryPick; objectName: 'bank-category-pick'; Layout.fillWidth: true; model: ['None'].concat(f.columns); Accessible.name: 'Category column' } }
                    Labelled {
                        visible: categoryPick.currentIndex > 0
                        label: 'Bank words'
                        Repeater {
                            model: f.categories
                            delegate: RowLayout {
                                id: cat
                                required property var modelData
                                required property int index
                                spacing: 10
                                Field { objectName: 'bank-word-' + cat.index; Layout.preferredWidth: 160; text: cat.modelData.bank; Accessible.name: 'Bank word'; onTextEdited: f.categories[cat.index].bank = text }
                                CategoryChoice {
                                    objectName: 'bank-key-' + cat.index
                                    Layout.preferredWidth: 200
                                    key: cat.modelData.key
                                    onActivated: index => f.categories[cat.index].key = pane.view.categories[index].key
                                }
                            }
                        }
                        SButton { objectName: 'bank-add-word'; text: 'Add word'; quiet: true; onClicked: f.categories = f.categories.concat([{bank: '', key: 'food:groceries'}]) }
                    }
                }
            }
            Footer { canSave: f.ready; busy: f.busy; onSaved: f.save() }
        }
    }

    // A receipt's lines typed once against its picture; the picture stays
    // their source.
    Component {
        id: receiptPane
        ColumnLayout {
            id: f
            property bool busy: false
            property var lines: [blank()]
            readonly property bool picture: ['png', 'jpg'].indexOf((pane.item || {}).kind) >= 0
            function blank() { return {item: '', quantity: '1', unit: 'item', amount: '', category: 'food:groceries'}; }
            // Typing changes a line in place: the rows are not rebuilt under the cursor.
            property int edits: 0
            function set(i, field, value) { lines[i][field] = value; edits++; }
            readonly property bool ready: edits >= 0 && !!merchantField.text.trim() && /^\d{4}-\d{2}-\d{2}$/.test(dayField.text.trim())
                                          && lines.length > 0 && lines.every(l => l.item.trim() && /^\d+(\.\d{1,2})?$/.test(l.amount.trim()) && Number(l.quantity) > 0)
            function save() {
                if (!ready || busy) return;
                busy = true;
                var day = dayField.text.trim(), merchant = merchantField.text.trim();
                pane.view.saveReceipt(pane.item, lines.map(l => ({merchant: merchant, day: day, item: l.item.trim(), quantity: Number(l.quantity),
                                                                  unit: l.unit.trim() || 'item', amount: l.amount.trim(), currency: pane.view.currency,
                                                                  category: l.category})), () => f.busy = false);
            }
            Component.onCompleted: { dayField.text = (pane.item || {}).day || Backend.today; merchantField.forceActiveFocus(); }
            spacing: 0
            Flickable {
                id: receiptScroll
                Layout.fillWidth: true; Layout.fillHeight: true
                clip: true
                contentWidth: width
                contentHeight: receiptBody.implicitHeight + 48
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: SScrollBar { }
                ColumnLayout {
                    id: receiptBody
                    x: 24; y: 24
                    width: receiptScroll.width - 48
                    spacing: 18
                    Image {
                        objectName: 'receipt-picture'
                        visible: f.picture
                        Layout.fillWidth: true
                        Layout.preferredHeight: f.picture ? Math.min(360, implicitHeight > 0 ? width * implicitHeight / Math.max(1, implicitWidth) : 360) : 0
                        fillMode: Image.PreserveAspectFit
                        horizontalAlignment: Image.AlignLeft
                        asynchronous: true
                        source: f.picture ? 'file://' + pane.view.intakePath((pane.item || {}).name) : ''
                    }
                    RowLayout {
                        Layout.fillWidth: true; spacing: 12
                        Labelled { label: 'Merchant'; Field { id: merchantField; objectName: 'receipt-merchant'; Layout.fillWidth: true; Accessible.name: 'Merchant' } }
                        Labelled { Layout.fillWidth: false; Layout.preferredWidth: 120; label: 'Day'; Field { id: dayField; objectName: 'receipt-day'; Layout.fillWidth: true; Accessible.name: 'Day' } }
                    }
                    Repeater {
                        model: f.lines
                        delegate: ColumnLayout {
                            id: line
                            required property var modelData
                            required property int index
                            Layout.fillWidth: true
                            spacing: 6
                            RowLayout {
                                Layout.fillWidth: true; spacing: 8
                                Field { objectName: 'receipt-item-' + line.index; Layout.fillWidth: true; placeholderText: 'Item'; text: line.modelData.item; Accessible.name: 'Item'; onTextEdited: f.set(line.index, 'item', text) }
                                Field { objectName: 'receipt-amount-' + line.index; Layout.preferredWidth: 90; placeholderText: 'Amount'; text: line.modelData.amount; Accessible.name: 'Amount'; onTextEdited: f.set(line.index, 'amount', text) }
                            }
                            RowLayout {
                                Layout.fillWidth: true; spacing: 8
                                Field { objectName: 'receipt-quantity-' + line.index; Layout.preferredWidth: 70; text: line.modelData.quantity; Accessible.name: 'Quantity'; onTextEdited: f.set(line.index, 'quantity', text) }
                                Field { objectName: 'receipt-unit-' + line.index; Layout.preferredWidth: 70; text: line.modelData.unit; Accessible.name: 'Unit'; onTextEdited: f.set(line.index, 'unit', text) }
                                CategoryChoice {
                                    objectName: 'receipt-category-' + line.index
                                    Layout.fillWidth: true
                                    key: line.modelData.category
                                    onActivated: index => f.set(line.index, 'category', pane.view.categories[index].key)
                                }
                                SButton { visible: f.lines.length > 1; iconName: 'close'; accessibleLabel: 'Remove line'; quiet: true; onClicked: { var l = f.lines.slice(); l.splice(line.index, 1); f.lines = l; } }
                            }
                        }
                    }
                    SButton { objectName: 'receipt-add-line'; text: 'Add line'; quiet: true; onClicked: f.lines = f.lines.concat([f.blank()]) }
                }
            }
            Footer { canSave: f.ready; busy: f.busy; onSaved: f.save() }
        }
    }

    // Cash, by hand, in one row.
    Component {
        id: cashPane
        ColumnLayout {
            id: f
            property bool busy: false
            property string category: 'food:groceries'
            readonly property bool ready: /^\d{4}-\d{2}-\d{2}$/.test(dayField.text.trim()) && !!merchantField.text.trim() && /^\d+(\.\d{1,2})?$/.test(amountField.text.trim())
            function save() {
                if (!ready || busy) return;
                busy = true;
                pane.view.saveCash({day: dayField.text.trim(), merchant: merchantField.text.trim(), amount: amountField.text.trim(),
                                    currency: pane.view.currency, category: category}, () => f.busy = false);
            }
            Component.onCompleted: { dayField.text = Backend.today; merchantField.forceActiveFocus(); }
            spacing: 0
            RowLayout {
                Layout.fillWidth: true; Layout.margins: 24; spacing: 8
                Labelled { Layout.fillWidth: false; Layout.preferredWidth: 104; label: 'Day'; Field { id: dayField; objectName: 'cash-day'; Layout.fillWidth: true; Accessible.name: 'Day' } }
                Labelled { label: 'Merchant'; Field { id: merchantField; objectName: 'cash-merchant'; Layout.fillWidth: true; Accessible.name: 'Merchant' } }
                Labelled { Layout.fillWidth: false; Layout.preferredWidth: 80; label: 'Amount'; Field { id: amountField; objectName: 'cash-amount'; Layout.fillWidth: true; Accessible.name: 'Amount'; onAccepted: f.save() } }
            }
            Labelled {
                Layout.leftMargin: 24; Layout.rightMargin: 24
                label: 'Category'
                CategoryChoice { objectName: 'cash-category'; Layout.fillWidth: true; key: f.category; onActivated: index => f.category = pane.view.categories[index].key }
            }
            Item { Layout.fillHeight: true }
            Footer { canSave: f.ready; busy: f.busy; onSaved: f.save() }
        }
    }

    // A week's lines of one category, each with its source by name; the
    // category changes in place.
    Component {
        id: linesPane
        ColumnLayout {
            id: f
            property var lines: []
            function load() { pane.view.loadLines((pane.item || {}).week, (pane.item || {}).category, l => f.lines = l); }
            Component.onCompleted: load()
            spacing: 0
            Flickable {
                id: linesScroll
                Layout.fillWidth: true; Layout.fillHeight: true
                clip: true
                contentWidth: width
                contentHeight: linesBody.implicitHeight + 48
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: SScrollBar { }
                ColumnLayout {
                    id: linesBody
                    x: 16; y: 16
                    width: linesScroll.width - 32
                    spacing: 6
                    Repeater {
                        model: f.lines
                        delegate: Item {
                            id: line
                            required property var modelData
                            required property int index
                            readonly property bool stocked: pane.view.isInStock(modelData)
                            readonly property string bankWord: modelData.category === 'other' || modelData.category === 'uncategorised' ? String(modelData.category_word || '') : ''
                            readonly property var actions: (line.stocked ? [{text: 'In stock', run: () => Backend.navigate('Hearth', 'Kitchen')}] : [])
                                                           .concat(line.bankWord ? [{text: line.bankWord, word: true}] : [])
                            objectName: 'line-' + index
                            Layout.fillWidth: true
                            implicitHeight: lineBody.implicitHeight + 8
                            MouseArea { id: lineFace; anchors.fill: parent; hoverEnabled: true; acceptedButtons: Qt.NoButton }
                            ColumnLayout {
                                id: lineBody
                                anchors.left: parent.left; anchors.right: parent.right; anchors.leftMargin: 8
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 2
                                RowLayout {
                                    Layout.fillWidth: true; spacing: 8
                                    LabelText { Layout.fillWidth: true; text: line.modelData.item || line.modelData.merchant || ''; elide: Text.ElideRight }
                                    SIcon { objectName: 'line-stock-' + line.index; visible: line.stocked; name: 'hearth'; color: Theme.accent; size: Theme.fontSizeSmall }
                                    LabelText { text: Backend.moneyText(line.modelData.amount, line.modelData.currency); font.family: Theme.mono }
                                    Item {
                                        implicitWidth: 28; implicitHeight: 28
                                        SButton {
                                            id: moreButton
                                            anchors.fill: parent
                                            objectName: 'line-more-' + line.index
                                            visible: line.actions.length > 0 && (lineFace.containsMouse || hovered || menu.visible)
                                            iconName: 'more'; accessibleLabel: 'More actions'; quiet: true
                                            focusPolicy: Qt.NoFocus
                                            onClicked: menu.open()
                                        }
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true; spacing: 8
                                    LabelText { Layout.preferredWidth: 80; text: Backend.longDate(line.modelData.date); color: Theme.muted; font.pixelSize: 12 }
                                    LabelText { objectName: 'line-source-' + line.index; Layout.fillWidth: true; text: pane.view.sourceName(line.modelData.source); color: Theme.muted; font.pixelSize: 12; elide: Text.ElideMiddle }
                                    CategoryChoice {
                                        objectName: 'line-category-' + line.index
                                        Layout.preferredWidth: 170
                                        key: line.modelData.category
                                        onActivated: index => pane.view.recategorise(line.modelData.id, pane.view.categories[index].key, () => f.load())
                                    }
                                }
                            }
                            SPopup {
                                id: menu
                                trigger: moreButton
                                alignRight: true
                                parent: Overlay.overlay
                                width: 180
                                padding: 6
                                contentItem: ColumnLayout {
                                    spacing: 2
                                    Repeater {
                                        model: line.actions
                                        delegate: MenuAction {
                                            required property var modelData
                                            objectName: 'line-action-' + line.index + '-' + (modelData.word ? 'word' : modelData.text)
                                            text: modelData.text
                                            enabled: !modelData.word
                                            onClicked: { menu.close(); if (modelData.run) modelData.run(); }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
