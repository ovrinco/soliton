pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Hearth, Finance: every dollar in. Income by period above the spending by
// category, week by week, as the engine reports them; a week's row opens its
// lines beside the page, each with its document named as its source. The
// intake list is the one way in; cash is typed in one row.
FocusScope {
    id: view
    objectName: 'finance-view'

    readonly property bool empty: Backend.financeLoaded && Backend.financeWeeks.length === 0 && income.length === 0

    // The pane beside the page: intake, bank, newBank, receipt, cash, lines; '' when closed.
    property string paneKind: ''
    property var paneItem: null
    property int paneSerial: 0
    function openPane(kind, item) { paneKind = kind; paneItem = item || null; paneSerial++; }
    function closePane() { paneKind = ''; paneItem = null; }

    property var categories: []
    property var documents: ({})
    property var intake: []
    property bool intakeLoaded: false
    property var income: []

    // The category's word as the engine gives it; a line with no word reads Uncategorised.
    function word(key) {
        if (key === 'uncategorised') return 'Uncategorised';
        var known = categories.find(c => c.key === key);
        return known ? known.word : Backend.categoryWord(key);
    }
    // New lines take the currency the spending already holds, else his own.
    readonly property string currency: {
        var seen = {};
        Backend.financeWeeks.forEach(w => w.rows.forEach(r => seen[r.currency] = (seen[r.currency] || 0) + 1));
        var best = Object.keys(seen).sort((a, b) => seen[b] - seen[a])[0];
        return best || 'CAD';
    }
    function intakePath(name) { return Backend.state.root + '/intake/finance/' + name; }
    function documentOf(file) { return file.document ? documents[file.document] || {lines: 0} : null; }
    function sourceName(source) {
        var text = String(source || '');
        if (text === 'cash') return 'Cash';
        var document = /^document:([^#]+)#/.exec(text);
        if (document) return (documents[document[1]] || {}).name || '';
        var statement = /^statement:([^:]+):/.exec(text);
        return statement ? statement[1] : '';
    }
    function isInStock(line) { return line.in_stock === true; }

    function reload() {
        Backend.loadFinance();
        loadDocuments();
        loadIncome();
    }
    function loadCategories() {
        Backend.send('finance-categories', {}, (r, ok) => { if (ok) categories = r.categories || []; }, true);
    }
    function loadDocuments(cb) {
        Backend.ledgerSend('finance-documents', {}, (r, ok) => {
            var map = {};
            if (ok) (r.documents || []).forEach(d => map[d.id] = d);
            documents = map;
            if (cb) cb();
        });
    }
    function loadIncome() {
        Backend.send('finance-income', {}, (r, ok) => income = ok ? r.lines || [] : [], true);
    }
    // Listing the folder is his act: the intake is read when its pane opens.
    function loadIntake() {
        loadDocuments(() => Backend.ledgerSend('finance-intake', {}, (r, ok) => {
            if (!ok) { Backend.message('Finance is not available right now', true); return; }
            intake = r.files || [];
            intakeLoaded = true;
        }));
    }
    function afterImport(added) {
        Backend.message(added + (added === 1 ? ' line in' : ' lines in'), false);
        reload();
        openPane('intake');
    }
    // A file of the list comes in: a CSV is read at once (a statement asks its
    // bank first), a picture, PDF or e-mail opens its lines.
    function importFile(file) {
        var go = document => {
            var item = {document: document, name: file.name, kind: file.kind, day: file.day};
            if (file.kind !== 'csv') { openPane('receipt', item); return; }
            Backend.ledgerSend('finance-import', {document: document, currency: currency}, (r, ok, error) => {
                if (ok) { afterImport(r.added); return; }
                if (/Choose the bank/.test(error || '')) openPane('bank', item);
                else Backend.failNotice('Could not import', error);
            });
        };
        if (file.document) { go(file.document); return; }
        Backend.ledgerSend('finance-document-register', {name: file.name, kind: file.kind}, (r, ok, error) => {
            if (!ok) { Backend.failNotice('Could not import', error); loadIntake(); return; }
            go(r.id);
        });
    }
    function importStatement(item, bank, done) {
        Backend.ledgerSend('finance-import', {document: item.document, bank: bank}, (r, ok, error) => {
            done();
            if (ok) { afterImport(r.added); return; }
            if (/no statement mapping/.test(error || '')) openPane('newBank', Object.assign({}, item, {bank: bank}));
            else Backend.failNotice('Could not import', error);
        });
    }
    function saveReceipt(item, lines, done) {
        Backend.ledgerSend('finance-receipt-lines', {document: item.document, lines: lines}, (r, ok, error) => {
            done();
            if (ok) afterImport((r.lines || []).length);
            else Backend.failNotice('Could not save the lines', error);
        });
    }
    function saveCash(args, done) {
        Backend.ledgerSend('finance-cash', args, (r, ok, error) => {
            done();
            if (!ok) { Backend.failNotice('Could not save the cash', error); return; }
            reload();
            closePane();
        });
    }
    function loadLines(week, category, cb) {
        Backend.send('finance-lines', {week: week, category: category}, (r, ok) => {
            if (!ok) { Backend.message('Finance is not available right now', true); cb([]); return; }
            cb((r.lines || []).filter(l => l.category === category && (l.week === week || !l.week)));
        }, true);
    }
    function recategorise(line, category, cb) {
        Backend.financeRecategorise(line, category, ok => { reload(); cb(ok); });
    }
    Component.onCompleted: { loadCategories(); reload(); }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        ColumnLayout {
            Layout.fillWidth: true; Layout.fillHeight: true
            spacing: 12

            RowLayout {
                Layout.fillWidth: true
                spacing: 4
                Item { Layout.fillWidth: true }
                SButton { objectName: 'finance-import'; text: 'Import'; quiet: true; onClicked: view.openPane('intake') }
                SButton { id: pageMore; objectName: 'finance-more'; iconName: 'more'; accessibleLabel: 'More actions'; quiet: true; onClicked: pageMenu.open() }
            }

            LabelText { visible: view.empty; text: 'No spending'; color: Theme.muted }
            Item { Layout.fillHeight: true; visible: !scroll.visible }

            Flickable {
                id: scroll
                visible: !view.empty && Backend.financeLoaded
                Layout.fillWidth: true; Layout.fillHeight: true
                clip: true
                contentWidth: width
                contentHeight: body.implicitHeight + 16
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: SScrollBar { }

                ColumnLayout {
                    id: body
                    width: scroll.width - 12
                    spacing: 20

                    ColumnLayout {
                        Layout.fillWidth: true
                        visible: view.income.length > 0
                        spacing: 2
                        LabelText { Layout.bottomMargin: 4; text: 'Income'; font.pixelSize: Theme.sectionTitleSize; font.weight: Font.Medium }
                        Repeater {
                            model: view.income
                            delegate: HearthRow {
                                required property var modelData
                                required property int index
                                objectName: 'income-' + index
                                Layout.fillWidth: true
                                opens: false
                                nameWidth: 220
                                name: modelData.period_start && modelData.period_end
                                      ? Backend.longDate(modelData.period_start) + ' to ' + Backend.longDate(modelData.period_end)
                                      : Backend.longDate(modelData.day)
                                cells: [
                                    {text: Backend.moneyText(modelData.net, modelData.currency), mono: true, right: true, width: 140},
                                    {text: modelData.employer || '', muted: true, width: 220}
                                ]
                            }
                        }
                    }

                    LabelText { text: 'Spending by category'; visible: Backend.financeWeeks.length > 0; font.pixelSize: Theme.sectionTitleSize; font.weight: Font.Medium }
                    Repeater {
                        model: Backend.financeWeeks
                        delegate: ColumnLayout {
                            id: week
                            required property var modelData
                            Layout.fillWidth: true
                            spacing: 2
                            LabelText { Layout.bottomMargin: 4; text: 'Week of ' + Backend.longDate(week.modelData.week); font.pixelSize: 14; font.weight: Font.Medium }
                            Repeater {
                                model: week.modelData.rows
                                delegate: HearthRow {
                                    required property var modelData
                                    objectName: 'spend-' + week.modelData.week + '-' + modelData.category
                                    Layout.fillWidth: true
                                    nameWidth: 220
                                    name: view.word(modelData.category)
                                    cells: [
                                        {text: Backend.moneyText(modelData.spent, modelData.currency), mono: true, right: true, width: 140},
                                        {text: modelData.lines + (Number(modelData.lines) === 1 ? ' line' : ' lines'), muted: true, small: true, width: 80}
                                    ]
                                    onActivated: view.openPane('lines', {week: week.modelData.week, category: modelData.category, currency: modelData.currency})
                                }
                            }
                        }
                    }
                }
            }
        }

        Rectangle { Layout.fillHeight: true; Layout.leftMargin: 12; Layout.preferredWidth: 1; color: Theme.line; visible: pane.visible }
        FinancePane {
            id: pane
            view: view
            visible: !!view.paneKind
            Layout.fillHeight: true
            Layout.preferredWidth: 440
            focus: visible
        }
    }
    SPopup {
        id: pageMenu
        trigger: pageMore
        alignRight: true
        parent: Overlay.overlay
        width: 180
        padding: 6
        contentItem: ColumnLayout {
            spacing: 2
            MenuAction { objectName: 'finance-action-cash'; text: 'Cash'; onClicked: { pageMenu.close(); view.openPane('cash'); } }
        }
    }
}
