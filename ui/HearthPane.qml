pragma ComponentBehavior: Bound
import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts

// The one right pane of Kitchen and Finance: a lot, a piece of equipment, a
// constraint, the batch plan, a statement import or a bank's set-up. Save and
// Cancel appear once something has changed; a refusal leaves the pane as it was.
Item {
    id: pane
    objectName: 'hearth-pane'
    readonly property string kind: Backend.hearthPane.kind
    readonly property var item: Backend.hearthPane.item

    function close() { Backend.closeHearthPane(); }
    function title() {
        switch (kind) {
        case 'newLot': return item && item.purchase ? 'To stock' : 'New lot';
        case 'lot': return item ? item.subject : '';
        case 'newEquipment': return 'New equipment';
        case 'equipment': return item ? item.name : '';
        case 'newConstraint': return 'New constraint';
        case 'constraint': return item ? subjectWord(item.subject) : '';
        case 'plan': return planLoader.item && planLoader.item.result ? planLoader.item.resultTitle : 'Batch plan';
        case 'import': return 'Import statement';
        case 'bank': return 'Set up this bank';
        case 'lines': return item ? Backend.categoryWord(item.category) + ' · Week of ' + Backend.longDate(item.week) : '';
        }
        return '';
    }
    function subjectWord(subject) {
        var words = {exclude: 'Exclude', required: 'Required', diet: 'Diet', portions_per_week: 'Portions a week', session_hours: 'Session length'};
        if (words[subject]) return words[subject];
        var text = String(subject || '').replace(/[^A-Za-z0-9]+/g, ' ').trim();
        return text.charAt(0).toUpperCase() + text.slice(1);
    }
    // A lot's quantity: a weight in one of the engine's mass units, or a count.
    readonly property var weightUnits: ['g', 'kg', 'lb', 'oz']
    // The picture a lot takes: an image in the intake folder, listed by the
    // engine, one row each by name and day, or one chosen on his disk; a
    // second click lets it go.
    function intakePicture(name) { return name ? 'intake/finance/' + name : ''; }
    Keys.onEscapePressed: close()

    component Label: LabelText { font.pixelSize: 12; color: Theme.muted }
    component Labelled: ColumnLayout {
        property alias label: caption.text
        Layout.fillWidth: true; spacing: 6
        Label { id: caption }
    }
    component Footer: RowLayout {
        id: footer
        property bool shown: false
        property bool canSave: true
        property bool busy: false
        signal saved()
        Layout.fillWidth: true
        Layout.margins: 16
        visible: shown
        Item { Layout.fillWidth: true }
        SButton { text: 'Cancel'; quiet: true; onClicked: pane.close() }
        SButton { objectName: 'pane-save'; text: 'Save'; quiet: true; busy: footer.busy; enabled: footer.canSave; onClicked: footer.saved() }
    }

    component PicturePick: ColumnLayout {
        id: pick
        // The picture argument the engine takes: an intake file by its
        // workspace path, or a file chosen on his disk by its own path.
        property string chosen: ''
        property string picked: ''
        property var files: []
        function pathFromUrl(url) {
            var value = String(url || '');
            if (value.indexOf('file://') === 0) value = value.slice(7);
            try { return decodeURIComponent(value); } catch (error) { return value; }
        }
        // The file dialog's choice: shown as a row and marked as the picture.
        function choose(url) { picked = pathFromUrl(url); chosen = picked; }
        // A new choice answers a refusal of the last one: its notice goes, so
        // it no longer covers Save.
        onChosenChanged: if (Backend.noticeError) { Backend.notice = ''; Backend.noticeError = false; }
        objectName: 'lot-picture-pick'
        Layout.fillWidth: true
        spacing: 0
        Component.onCompleted: Backend.send('finance-intake', {}, (r, ok) => {
            pick.files = ok ? (r.files || []).filter(f => f.kind === 'jpg' || f.kind === 'png') : [];
        }, true)
        Repeater {
            model: pick.files
            delegate: HearthRow {
                required property var modelData
                required property int index
                objectName: 'lot-picture-' + index
                Layout.fillWidth: true
                name: modelData.name
                nameWidth: 200
                cells: [{text: Backend.longDate(modelData.day), muted: true, small: true, width: 90},
                        {text: '', mark: pick.chosen === pane.intakePicture(modelData.name), markColor: Theme.accent, width: 12}]
                onActivated: pick.chosen = pick.chosen === pane.intakePicture(modelData.name) ? '' : pane.intakePicture(modelData.name)
            }
        }
        HearthRow {
            objectName: 'lot-picture-picked'
            visible: !!pick.picked
            Layout.fillWidth: true
            picture: pick.picked
            pictureSlot: true
            name: pick.picked.slice(pick.picked.lastIndexOf('/') + 1)
            nameWidth: 200
            cells: [{text: '', mark: pick.chosen === pick.picked, markColor: Theme.accent, width: 12}]
            onActivated: pick.chosen = pick.chosen === pick.picked ? '' : pick.picked
        }
        SButton {
            objectName: 'lot-picture-choose'
            Layout.topMargin: pick.files.length || pick.picked ? 6 : 0
            text: 'Choose'
            quiet: true
            onClicked: pictureFile.open()
        }
        FileDialog {
            id: pictureFile
            title: 'Choose a picture'
            fileMode: FileDialog.OpenFile
            nameFilters: ['Pictures (*.png *.jpg *.jpeg *.webp)']
            currentFolder: StandardPaths.writableLocation(StandardPaths.PicturesLocation)
            onAccepted: pick.choose(selectedFile)
        }
    }
    component QuantityFields: RowLayout {
        id: qty
        property alias weight: weightField.text
        property alias count: countField.text
        readonly property string unit: pane.weightUnits[unitChoice.currentIndex]
        readonly property bool given: weight !== '' || count !== ''
        // The quantity as the engine takes it: the weight first, else the count.
        function quantity() { return weight !== '' ? {quantity: Number(weight), unit: unit} : {quantity: Number(count), unit: 'pieces'}; }
        spacing: 12
        Labelled {
            label: 'Weight'; Layout.fillWidth: false
            RowLayout {
                spacing: 6
                Field { id: weightField; objectName: 'lot-weight'; Layout.preferredWidth: 110; font.family: Theme.mono; enabled: countField.text === ''; validator: DoubleValidator { bottom: 0; notation: DoubleValidator.StandardNotation } Accessible.name: 'Weight' }
                Choice { id: unitChoice; objectName: 'lot-weight-unit'; Layout.preferredWidth: 84; model: pane.weightUnits; enabled: countField.text === ''; Accessible.name: 'Unit' }
            }
        }
        Labelled {
            label: 'Count'; Layout.fillWidth: false; Layout.preferredWidth: 110
            Field { id: countField; objectName: 'lot-count'; Layout.fillWidth: true; font.family: Theme.mono; enabled: weightField.text === ''; validator: DoubleValidator { bottom: 0; notation: DoubleValidator.StandardNotation } Accessible.name: 'Count' }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0
        Item {
            Layout.fillWidth: true; Layout.preferredHeight: 52
            RowLayout {
                anchors.fill: parent; anchors.leftMargin: 24; anchors.rightMargin: 16; spacing: 4
                LabelText { Layout.fillWidth: true; text: pane.title(); font.pixelSize: 14; font.weight: Font.Medium; elide: Text.ElideRight }
                SButton { iconName: 'close'; accessibleLabel: 'Close'; quiet: true; onClicked: pane.close() }
            }
        }
        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.line }
        Loader {
            id: planLoader
            Layout.fillWidth: true; Layout.fillHeight: true
            sourceComponent: {
                switch (pane.kind) {
                case 'newLot': return newLotPane;
                case 'lot': return lotPane;
                case 'newEquipment': case 'equipment': return equipmentPane;
                case 'newConstraint': case 'constraint': return constraintPane;
                case 'plan': return planPane;
                case 'import': return importPane;
                case 'bank': return bankPane;
                case 'lines': return linesPane;
                }
                return null;
            }
            // A new opening starts the pane afresh.
            Connections {
                target: Backend
                function onHearthPaneSerialChanged() { planLoader.active = false; planLoader.active = true; }
            }
        }
    }

    Component {
        id: newLotPane
        ColumnLayout {
            id: f
            // From a purchase line: the engine names the lot after the line
            // and the line is its source; the day is the day it was bought.
            readonly property var purchase: (pane.item || {}).purchase || null
            property bool saving: false
            readonly property bool dirty: !!purchase || nameField.text !== '' || quantity.given || !!expiry.dateValue || whereField.text !== '' || !!picture.chosen
            readonly property bool ready: (!!purchase || !!nameField.text.trim()) && quantity.given
            function done(ok) { saving = false; Backend.loadKitchen(); if (ok) pane.close(); }
            function withPicture(lot) {
                if (!picture.chosen) { done(true); return; }
                Backend.send('kitchen-stock-picture', {lot: lot, picture: picture.chosen, day: Backend.today}, (r, ok, error) => {
                    if (!ok) Backend.failNotice('Could not save the lot', error);
                    done(ok);
                }, true);
            }
            // A chosen picture is checked first, so a file that is not one
            // leaves nothing written: no lot, no copy.
            function save() {
                if (!ready || saving) return;
                saving = true;
                if (!picture.chosen) { write(); return; }
                Backend.send('kitchen-picture-check', {picture: picture.chosen}, (r, ok, error) => {
                    if (!ok) { Backend.failNotice('Could not save the lot', error); saving = false; return; }
                    write();
                }, true);
            }
            function write() {
                var args = quantity.quantity();
                if (expiry.dateValue) args.expires_on = expiry.dateValue;
                if (whereField.text.trim()) args.location = whereField.text.trim();
                if (purchase) {
                    args.line = purchase.line;
                    args.day = bought.dateValue || purchase.day;
                    Backend.ledgerSend('kitchen-lot-from-purchase', args, (r, ok, error) => {
                        if (!ok) { Backend.failNotice('Could not save the lot', error); done(false); return; }
                        withPicture(r.id);
                    });
                    return;
                }
                args.subject = nameField.text.trim();
                args.source = 'typed';
                args.day = Backend.today;
                if (picture.chosen) args.picture = picture.chosen;
                Backend.ledgerSend('kitchen-stock-add', args, (r, ok, error) => {
                    if (!ok) Backend.failNotice('Could not save the lot', error);
                    done(ok);
                });
            }
            Component.onCompleted: {
                if (purchase) { nameField.text = purchase.subject || ''; bought.dateValue = purchase.day || ''; }
                else nameField.forceActiveFocus();
            }
            spacing: 0
            Flickable {
                id: lotScroll
                Layout.fillWidth: true; Layout.fillHeight: true
                clip: true
                contentWidth: width
                contentHeight: lotBody.implicitHeight + 48
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: SScrollBar { }
                ColumnLayout {
                    id: lotBody
                    x: 24; y: 24
                    width: lotScroll.width - 48
                    spacing: 18
                    Labelled { label: 'Name'; Field { id: nameField; objectName: 'lot-name'; Layout.fillWidth: true; enabled: !f.purchase; Accessible.name: 'Name'; onAccepted: f.save() } }
                    Labelled { visible: !!f.purchase; label: 'Bought'; SDateField { id: bought; objectName: 'lot-bought'; placeholderText: ''; accessibleName: 'Bought'; showClearButton: false; onDateEdited: iso => { if (iso) dateValue = iso; } } }
                    QuantityFields { id: quantity }
                    Labelled { label: 'Goes off'; SDateField { id: expiry; objectName: 'lot-expiry'; placeholderText: ''; accessibleName: 'Goes off'; onDateEdited: iso => dateValue = iso } }
                    Labelled { label: 'Where'; Field { id: whereField; objectName: 'lot-where'; Layout.fillWidth: true; Accessible.name: 'Where'; onAccepted: f.save() } }
                    Labelled { label: 'Picture'; PicturePick { id: picture } }
                }
            }
            Footer { shown: f.dirty; canSave: f.ready; busy: f.saving; onSaved: f.save() }
        }
    }

    Component {
        id: lotPane
        ColumnLayout {
            id: f
            readonly property var lot: pane.item || ({})
            property bool saving: false
            readonly property string onHandWas: Backend.amountText(lot.quantity, '')
            readonly property bool usedSet: usedField.text !== ''
            readonly property bool onHandSet: onHandField.text !== onHandWas
            readonly property bool expirySet: (expiry.dateValue || '') !== (lot.expires_on || '')
            readonly property bool whereSet: whereField.text.trim() !== (lot.location || '') && !!whereField.text.trim()
            readonly property bool dirty: usedSet || onHandSet || expirySet || whereSet || !!picture.chosen
            function save() {
                if (saving || !dirty) return;
                var changes = {};
                if (onHandSet && onHandField.text !== '') changes.quantity = Number(onHandField.text);
                else if (usedSet) changes.used = Number(usedField.text);
                if (expirySet && expiry.dateValue) changes.expires_on = expiry.dateValue;
                if (whereSet) changes.location = whereField.text.trim();
                saving = true;
                var rest = id => Backend.kitchenChangeLot(id, changes, ok => { saving = false; if (ok) pane.close(); });
                if (!picture.chosen) { rest(lot.id); return; }
                // The picture first: it supersedes the lot, and the other
                // changes are written on the lot it leaves current.
                Backend.send('kitchen-stock-picture', {lot: lot.id, picture: picture.chosen, day: Backend.today}, (r, ok, error) => {
                    if (!ok) { Backend.failNotice('Could not save the lot', error); saving = false; Backend.loadKitchen(); return; }
                    rest(r.id);
                }, true);
            }
            Component.onCompleted: { onHandField.text = onHandWas; expiry.dateValue = lot.expires_on || ''; whereField.text = lot.location || ''; usedField.forceActiveFocus(); }
            spacing: 0
            Flickable {
                id: lotScroll
                Layout.fillWidth: true; Layout.fillHeight: true
                clip: true
                contentWidth: width
                contentHeight: lotBody.implicitHeight + 48
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: SScrollBar { }
                ColumnLayout {
                    id: lotBody
                    x: 24; y: 24
                    width: lotScroll.width - 48
                    spacing: 18
                    Rectangle {
                        visible: !!f.lot.picture
                        implicitWidth: 120; implicitHeight: 120
                        color: Theme.raised
                        radius: Theme.cornerRadius
                        clip: true
                        Image {
                            anchors.fill: parent
                            source: f.lot.picture ? 'file://' + f.lot.picture : ''
                            sourceSize: Qt.size(240, 240)
                            fillMode: Image.PreserveAspectCrop
                            asynchronous: true
                            cache: false
                        }
                    }
                    Labelled {
                        label: 'Used'
                        RowLayout { spacing: 10
                            Field { id: usedField; objectName: 'lot-used'; Layout.preferredWidth: 120; validator: DoubleValidator { bottom: 0; notation: DoubleValidator.StandardNotation } Accessible.name: 'Used'; onAccepted: f.save() }
                            LabelText { text: f.lot.unit || ''; color: Theme.muted } }
                    }
                    Labelled {
                        label: 'On hand'
                        RowLayout { spacing: 10
                            Field { id: onHandField; objectName: 'lot-onhand'; Layout.preferredWidth: 120; validator: DoubleValidator { bottom: 0; notation: DoubleValidator.StandardNotation } Accessible.name: 'On hand'; onAccepted: f.save() }
                            LabelText { text: f.lot.unit || ''; color: Theme.muted } }
                    }
                    Labelled { label: 'Goes off'; SDateField { id: expiry; placeholderText: ''; accessibleName: 'Goes off'; showClearButton: false; onDateEdited: iso => { if (iso) dateValue = iso; } } }
                    Labelled { label: 'Where'; Field { id: whereField; objectName: 'lot-where-edit'; Layout.fillWidth: true; Accessible.name: 'Where'; onAccepted: f.save() } }
                    Labelled { label: 'Picture'; PicturePick { id: picture } }
                }
            }
            Footer { shown: f.dirty; canSave: !f.onHandSet || onHandField.text !== ''; busy: f.saving; onSaved: f.save() }
        }
    }

    Component {
        id: equipmentPane
        ColumnLayout {
            id: f
            readonly property var piece: pane.kind === 'equipment' ? pane.item : null
            property bool saving: false
            readonly property string countWas: piece ? Backend.amountText(piece.count, '') : ''
            readonly property bool dirty: piece ? countField.text !== countWas || capacityField.text !== (piece.capacity || '')
                                                : nameField.text !== '' || countField.text !== '' || capacityField.text !== ''
            readonly property bool ready: (!!piece || !!nameField.text.trim()) && Number(countField.text) >= 1
            function save() {
                if (!ready || saving) return;
                saving = true;
                Backend.kitchenSetEquipment(piece ? piece.name : nameField.text.trim(), Number(countField.text), capacityField.text.trim(),
                                            ok => { saving = false; if (ok) pane.close(); });
            }
            Component.onCompleted: {
                if (piece) { countField.text = countWas; capacityField.text = piece.capacity || ''; countField.forceActiveFocus(); }
                else nameField.forceActiveFocus();
            }
            spacing: 0
            ColumnLayout {
                Layout.fillWidth: true; Layout.margins: 24; spacing: 18
                Labelled { visible: !f.piece; label: 'Name'; Field { id: nameField; objectName: 'equipment-name'; Layout.fillWidth: true; Accessible.name: 'Name' } }
                Labelled { label: 'Count'; Field { id: countField; objectName: 'equipment-count'; Layout.preferredWidth: 120; validator: IntValidator { bottom: 1; top: 999 } Accessible.name: 'Count' } }
                Labelled { label: 'Capacity'; Field { id: capacityField; objectName: 'equipment-capacity'; Layout.preferredWidth: 180; Accessible.name: 'Capacity'; onAccepted: f.save() } }
            }
            Item { Layout.fillHeight: true }
            Footer { shown: f.dirty; canSave: f.ready; busy: f.saving; onSaved: f.save() }
        }
    }

    Component {
        id: constraintPane
        ColumnLayout {
            id: f
            readonly property var entry: pane.kind === 'constraint' ? pane.item : null
            property bool saving: false
            readonly property bool hasValue: !!entry && entry.value !== null && entry.value !== undefined
            readonly property bool dirty: entry
                ? valueField.text !== (hasValue ? entry.value : '') || amountField.text !== (hasValue ? '' : Backend.amountText(entry.quantity, ''))
                  || unitField.text !== (hasValue ? '' : entry.unit || '')
                : nameField.text !== '' || valueField.text !== '' || amountField.text !== '' || unitField.text !== ''
            readonly property bool ready: (!!entry || !!nameField.text.trim()) && (!!valueField.text.trim() || amountField.text !== '')
            function save() {
                if (!ready || saving) return;
                var e = {subject: entry ? entry.subject : nameField.text.trim()};
                if (valueField.text.trim()) e.value = valueField.text.trim();
                else { e.quantity = Number(amountField.text); e.unit = unitField.text.trim(); }
                saving = true;
                Backend.kitchenSetProfile([e], ok => { saving = false; if (ok) pane.close(); });
            }
            Component.onCompleted: {
                if (entry) {
                    if (hasValue) valueField.text = entry.value;
                    else { amountField.text = Backend.amountText(entry.quantity, ''); unitField.text = entry.unit || ''; }
                    valueField.forceActiveFocus();
                } else nameField.forceActiveFocus();
            }
            spacing: 0
            ColumnLayout {
                Layout.fillWidth: true; Layout.margins: 24; spacing: 18
                Labelled { visible: !f.entry; label: 'Name'; Field { id: nameField; objectName: 'constraint-name'; Layout.fillWidth: true; Accessible.name: 'Name' } }
                Labelled { label: 'Value'; Field { id: valueField; objectName: 'constraint-value'; Layout.fillWidth: true; enabled: amountField.text === ''; Accessible.name: 'Value'; onAccepted: f.save() } }
                RowLayout {
                    spacing: 12
                    Labelled { label: 'Amount'; Layout.preferredWidth: 120; Field { id: amountField; objectName: 'constraint-amount'; Layout.fillWidth: true; enabled: valueField.text === ''; validator: DoubleValidator { notation: DoubleValidator.StandardNotation } Accessible.name: 'Amount' } }
                    Labelled { label: 'Unit'; Layout.preferredWidth: 180; Field { id: unitField; objectName: 'constraint-unit'; Layout.fillWidth: true; enabled: valueField.text === ''; Accessible.name: 'Unit' } }
                }
            }
            Item { Layout.fillHeight: true }
            Footer { shown: f.dirty; canSave: f.ready; busy: f.saving; onSaved: f.save() }
        }
    }

    Component {
        id: planPane
        ColumnLayout {
            id: f
            property string day: Backend.today
            // How many days of meals the batch covers; kept with the draft.
            property string coverage: '7'
            property var dishes: [f.newDish()]
            property int revision: 0
            property bool saving: false
            property var result: null
            property var written: []
            readonly property var phases: ['Prep', 'First cook', 'Second cook']
            readonly property var lots: Backend.kitchenStock
            readonly property bool ready: { revision; return Number(coverage) >= 1 && dishes.length > 0 && dishes.every(d => !!d.name.trim() && Number(d.portions) > 0); }
            // His setting: the local model drafts the dishes only while it is on.
            readonly property bool localDraft: Backend.prefs.kitchenDraft === true
            // The engine's refusal of a draft, as one word at the foot.
            property string refusal: ''
            readonly property string resultTitle: {
                var p = day.split('-');
                return new Date(Number(p[0]), Number(p[1]) - 1, Number(p[2])).toLocaleDateString(Qt.locale('en_GB'), 'dddd d MMMM');
            }
            // A plan already written for the day is read back and shown as written;
            // a plan just written is read back for its dishes and coverage.
            property var existing: null
            function readBack(fresh) {
                var d = day;
                Backend.kitchenPlanFor(d, plan => {
                    if (d !== day || saving) return;
                    existing = fresh ? null : plan;
                    if (plan) {
                        written = plan.dishes.map(x => ({id: x.id, meal: x.meal || null, plan: plan.plan_id, name: x.name, portions: x.portions, phase: x.phase, standing: x.standing || ''}));
                        result = Object.assign({}, result && result.plan_id === plan.plan_id ? result : {notes: [], goes_off: null}, {coverage_days: plan.coverage_days});
                    } else if (result && !result.plan_id) { result = null; written = []; }
                });
            }
            onDayChanged: { keep(); readBack(); }
            onDishesChanged: keep()
            onCoverageChanged: keep()
            // 13.11 3a: the draft is kept in Backend while the plan is open, and
            // a reopening takes it back.
            property bool restored: false
            function keep() { if (restored && !result) Backend.planDraft = JSON.parse(JSON.stringify({day: day, coverage: coverage, dishes: dishes})); }
            Component.onCompleted: {
                var d = Backend.planDraft;
                if (d) { day = d.day; if (d.coverage) coverage = d.coverage; dishes = JSON.parse(JSON.stringify(d.dishes)); }
                restored = true;
                keep();
                readBack();
            }
            function newDish() { return {name: '', portions: '', phase: 0, equipment: [], uses: []}; }
            function touch() { revision++; keep(); }
            function addDish() { dishes = dishes.concat([newDish()]); }
            function removeDish(i) { var d = dishes.slice(); d.splice(i, 1); dishes = d; }
            function addUse(i) { if (!lots.length) return; var d = dishes.slice(); d[i].uses = d[i].uses.concat([{lot: lots[0].id, qty: ''}]); dishes = d; }
            function write() {
                if (!ready || saving) return;
                var sent = dishes.map(d => ({
                    name: d.name.trim(), portions: Number(d.portions), phase: d.phase, equipment: d.equipment.slice(),
                    uses: d.uses.filter(u => u.lot && Number(u.qty) > 0).map(u => ({lot: u.lot, qty: Number(u.qty)}))
                }));
                saving = true;
                refusal = '';
                Backend.ledgerSend('kitchen-batch-plan', {day: day, horizon_days: Backend.kitchenSpan, coverage_days: Number(coverage), dishes: sent}, (r, ok, error) => {
                    saving = false;
                    if (!ok) {
                        var text = String(error || '');
                        if (/is already written/.test(text)) Backend.message('A plan for this day is already written', true);
                        else if (/no current lot/.test(text)) Backend.message('Could not write the plan · an ingredient is no longer in stock', true);
                        else Backend.failNotice('Could not write the plan', error);
                        return;
                    }
                    Backend.loadKitchen();
                    written = sent;
                    result = r;
                    Backend.planDraft = null;
                    existing = null;
                    readBack(true);
                });
            }
            // The local model drafts the dishes from the stock, the equipment
            // and the profile over the coverage; the engine writes them as a
            // draft, each kept at its standing, for him to judge.
            function draft() {
                if (saving || !localDraft || Number(coverage) < 1) return;
                saving = true;
                refusal = '';
                Backend.ledgerSend('kitchen-draft', {day: day, coverage_days: Number(coverage)}, (r, ok, error) => {
                    saving = false;
                    if (!ok) { refusal = 'Refused'; return; }
                    Backend.loadKitchen();
                    result = r;
                    Backend.planDraft = null;
                    existing = null;
                    readBack(true);
                });
            }
            spacing: 0

            Flickable {
                id: planScroll
                Layout.fillWidth: true; Layout.fillHeight: true
                clip: true
                contentWidth: width
                contentHeight: planBody.implicitHeight + 48
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: SScrollBar { }
                ColumnLayout {
                    id: planBody
                    x: 24; y: 24
                    width: planScroll.width - 48
                    spacing: 18

                    // Before writing: the day, the look-ahead and the dishes.
                    Labelled {
                        visible: !f.result || !!f.existing
                        label: 'Day'
                        SDateField { objectName: 'plan-day'; dateValue: f.day; placeholderText: ''; accessibleName: 'Day'; showClearButton: false; onDateEdited: iso => { if (iso) f.day = iso; } }
                    }
                    Labelled {
                        visible: !f.result
                        label: 'Coverage'
                        RowLayout {
                            spacing: 10
                            Field { objectName: 'plan-coverage'; Layout.preferredWidth: 90; font.family: Theme.mono; text: f.coverage; validator: IntValidator { bottom: 1; top: 365 } Accessible.name: 'Coverage'; onTextEdited: { f.coverage = text; f.touch(); } }
                            LabelText { text: 'days'; color: Theme.muted }
                        }
                    }
                    RowLayout {
                        visible: !f.result
                        spacing: 10
                        SettingsSwitch {
                            objectName: 'plan-local-draft'
                            checked: f.localDraft
                            Accessible.name: 'Local draft'
                            onClicked: Backend.setting('kitchenDraft', !f.localDraft)
                        }
                        LabelText { text: 'Local draft' }
                    }
                    LabelText { visible: !f.result; text: 'Dishes'; font.pixelSize: 14; font.weight: Font.Medium }
                    Repeater {
                        model: f.result ? [] : f.dishes
                        delegate: ColumnLayout {
                            id: dish
                            required property var modelData
                            required property int index
                            Layout.fillWidth: true
                            spacing: 10
                            Rectangle { Layout.fillWidth: true; height: 1; color: Theme.line }
                            RowLayout {
                                Layout.fillWidth: true
                                Labelled {
                                    label: 'Name'
                                    Field { objectName: 'dish-name-' + dish.index; Layout.fillWidth: true; text: dish.modelData.name; Accessible.name: 'Name'; onTextEdited: { f.dishes[dish.index].name = text; f.touch(); } }
                                }
                                SButton { Layout.alignment: Qt.AlignBottom; visible: f.dishes.length > 1; text: 'Remove dish'; quiet: true; onClicked: f.removeDish(dish.index) }
                            }
                            RowLayout {
                                spacing: 12
                                Labelled {
                                    label: 'Portions'; Layout.preferredWidth: 100
                                    Field { objectName: 'dish-portions-' + dish.index; Layout.fillWidth: true; text: dish.modelData.portions; validator: DoubleValidator { bottom: 0; notation: DoubleValidator.StandardNotation } Accessible.name: 'Portions'; onTextEdited: { f.dishes[dish.index].portions = text; f.touch(); } }
                                }
                                Labelled {
                                    label: 'Phase'; Layout.preferredWidth: 180
                                    Choice { objectName: 'dish-phase-' + dish.index; Layout.fillWidth: true; model: f.phases; currentIndex: dish.modelData.phase; Accessible.name: 'Phase'; onActivated: index => { f.dishes[dish.index].phase = index; f.keep(); } }
                                }
                            }
                            Labelled {
                                visible: Backend.kitchenEquipment.length > 0
                                label: 'Equipment'
                                Flow {
                                    Layout.fillWidth: true; spacing: 12
                                    Repeater {
                                        model: Backend.kitchenEquipment
                                        delegate: SCheckBox {
                                            required property var modelData
                                            objectName: 'dish-' + dish.index + '-equipment-' + modelData.name
                                            text: modelData.name
                                            checked: dish.modelData.equipment.indexOf(modelData.name) >= 0
                                            onToggled: {
                                                var list = f.dishes[dish.index].equipment.filter(n => n !== modelData.name);
                                                if (checked) list.push(modelData.name);
                                                f.dishes[dish.index].equipment = list;
                                                f.keep();
                                            }
                                        }
                                    }
                                }
                            }
                            Labelled {
                                label: 'Uses'
                                Repeater {
                                    model: dish.modelData.uses
                                    delegate: RowLayout {
                                        id: use
                                        required property var modelData
                                        required property int index
                                        spacing: 10
                                        readonly property var pick: f.lots.find(l => l.id === modelData.lot) || null
                                        Choice {
                                            objectName: 'dish-' + dish.index + '-use-' + use.index
                                            Layout.preferredWidth: 220
                                            model: f.lots.map(l => l.subject + ' · ' + Backend.amountText(l.quantity, l.unit))
                                            currentIndex: Math.max(0, f.lots.findIndex(l => l.id === use.modelData.lot))
                                            Accessible.name: 'Uses'
                                            onActivated: index => { f.dishes[dish.index].uses[use.index].lot = f.lots[index].id; f.dishes = f.dishes.slice(); }
                                        }
                                        Field { objectName: 'dish-' + dish.index + '-qty-' + use.index; Layout.preferredWidth: 90; text: use.modelData.qty; validator: DoubleValidator { bottom: 0; notation: DoubleValidator.StandardNotation } Accessible.name: 'Amount'; onTextEdited: { f.dishes[dish.index].uses[use.index].qty = text; f.keep(); } }
                                        LabelText { text: use.pick ? use.pick.unit || '' : ''; color: Theme.muted }
                                    }
                                }
                                SButton { objectName: 'dish-add-use-' + dish.index; visible: f.lots.length > 0; text: 'Add ingredient'; quiet: true; onClicked: f.addUse(dish.index) }
                            }
                        }
                    }
                    SButton { objectName: 'plan-add-dish'; visible: !f.result; text: 'Add dish'; quiet: true; onClicked: f.addDish() }

                    // After writing: the coverage and portions, the plan as written,
                    // the lots it draws on, the profile it was read against and
                    // what goes off after it. Closing the pane is the end of it.
                    // The portions planned are marked when short of what the
                    // coverage at the profile's portions a week wants.
                    Repeater {
                        model: {
                            var r = f.result;
                            if (!r) return [];
                            var rows = [];
                            if (r.coverage_days) rows.push({name: 'Coverage', cells: [{text: r.coverage_days + ' days', width: 150}]});
                            if (r.plan_id) {
                                var week = (r.constraints || []).find(c => c.subject === 'portions_per_week' && c.quantity !== null && c.quantity !== undefined);
                                var wanted = week && r.coverage_days ? Math.round(week.quantity * r.coverage_days / 7 * 10) / 10 : null;
                                rows.push({name: 'Portions', cells: [
                                    {text: Backend.amountText(r.portions, ''), mono: true, width: 60, mark: wanted !== null && r.portions + 1e-9 < wanted},
                                    {text: wanted !== null ? 'of ' + Backend.amountText(wanted, '') : '', muted: true, width: 80}]});
                            }
                            return rows;
                        }
                        delegate: HearthRow {
                            required property var modelData
                            Layout.fillWidth: true
                            opens: false
                            name: modelData.name
                            nameWidth: 120
                            cells: modelData.cells
                        }
                    }
                    Repeater {
                        model: f.result ? f.written : []
                        delegate: HearthRow {
                            required property var modelData
                            Layout.fillWidth: true
                            opens: false
                            name: modelData.name
                            // Narrow enough that the row and its menu fit the pane.
                            nameWidth: 140
                            // A dish of a plan read back can be cooked once: the meal is
                            // written and its uses are taken from stock (T028).
                            actions: modelData.id && !modelData.meal ? [{text: 'Cooked', run: () => Backend.kitchenCook(modelData.plan, modelData.id, ok => { if (ok) f.readBack(); })}] : []
                            // A drafted dish carries the muted mark until he takes it as his.
                            cells: [
                                {text: Backend.amountText(modelData.portions, ''), mono: true, right: true, width: 60},
                                {text: f.phases[modelData.phase] || '', muted: true, width: 90},
                                {text: '', mark: modelData.standing === 'inferred', markColor: Theme.muted, width: 10}
                            ]
                        }
                    }
                    // 13.11: the lots the plan draws on, soonest to go off first, each
                    // going off within the look-ahead marked so.
                    LabelText {
                        visible: !!f.result && (f.result.ingredients || []).length > 0
                        text: 'Uses'; font.pixelSize: 14; font.weight: Font.Medium
                    }
                    Repeater {
                        model: f.result ? (f.result.ingredients || []) : []
                        delegate: HearthRow {
                            required property var modelData
                            Layout.fillWidth: true
                            opens: false
                            name: modelData.subject
                            nameWidth: 96
                            cells: [
                                {text: Backend.amountText(modelData.quantity, modelData.unit), mono: true, right: true, width: 60},
                                {text: modelData.expires_on ? Backend.longDate(modelData.expires_on) : '', width: 84, small: true, mark: !!modelData.goes_off_soon}
                            ]
                        }
                    }
                    // 13.11: the cook profile the plan was read against, in the words of 13.3.
                    LabelText {
                        visible: !!f.result && (f.result.constraints || []).length > 0
                        text: 'Profile'; font.pixelSize: 14; font.weight: Font.Medium
                    }
                    Repeater {
                        model: {
                            var order = {portions_per_week: 0, session_hours: 1, diet: 2};
                            return (f.result ? (f.result.constraints || []) : []).slice()
                                .sort((a, b) => (a.subject in order ? order[a.subject] : 3) - (b.subject in order ? order[b.subject] : 3))
                                .map(c => ({
                                    entry: c,
                                    name: pane.subjectWord(c.subject),
                                    value: c.value !== null && c.value !== undefined && c.value !== '' ? String(c.value)
                                        : c.subject === 'session_hours' ? Backend.amountText(c.quantity, '') + ' hours'
                                        : Backend.amountText(c.quantity, c.subject === 'portions_per_week' ? '' : (c.unit || ''))
                                }));
                        }
                        // The diet is adjusted from the plan it shaped, through its row.
                        delegate: HearthRow {
                            required property var modelData
                            Layout.fillWidth: true
                            opens: false
                            name: modelData.name
                            nameWidth: 120
                            cells: [{text: modelData.value, width: 150}]
                            actions: modelData.entry.subject === 'diet'
                                ? [{text: 'Adjust diet', run: () => Backend.openHearthPane('constraint', Backend.kitchenProfile.find(e => e.subject === 'diet') || modelData.entry)}] : []
                        }
                    }
                    LabelText {
                        visible: !!f.result && ((f.result.goes_off || {}).rows || []).length > 0
                        text: 'Goes off'; font.pixelSize: 14; font.weight: Font.Medium
                    }
                    // The column words once, above the rows, so every row fits the pane.
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: 8; Layout.rightMargin: 4
                        visible: !!f.result && ((f.result.goes_off || {}).rows || []).length > 0
                        spacing: 8
                        Item { Layout.preferredWidth: 64 }
                        Repeater {
                            model: [{t: 'On hand', w: 52, r: true}, {t: 'Goes off', w: 66}, {t: 'Planned', w: 46, r: true}, {t: 'Unplanned', w: 58, r: true}]
                            delegate: LabelText {
                                required property var modelData
                                Layout.preferredWidth: modelData.w
                                text: modelData.t; font.pixelSize: 12; color: Theme.muted
                                horizontalAlignment: modelData.r ? Text.AlignRight : Text.AlignLeft
                                // Right-aligned words may run into the gap beside them rather than be cut.
                                elide: Text.ElideNone
                            }
                        }
                        Item { Layout.fillWidth: true }
                    }
                    Repeater {
                        model: f.result ? ((f.result.goes_off || {}).rows || []).map(row => {
                            var o = {}; (f.result.goes_off.columns || []).forEach((c, i) => o[c] = row[i]); return o; }) : []
                        delegate: HearthRow {
                            required property var modelData
                            Layout.fillWidth: true
                            opens: false
                            name: modelData.subject
                            nameWidth: 64
                            cells: [
                                {text: Backend.amountText(modelData.on_hand, modelData.unit), mono: true, right: true, width: 52},
                                {text: Backend.longDate(modelData.expires_on), width: 66, small: true},
                                {text: Backend.amountText(modelData.planned, modelData.unit), muted: true, small: true, right: true, width: 46},
                                // A lot no plan draws from is marked.
                                {text: Backend.amountText(modelData.unplanned, modelData.unit), muted: true, small: true, right: true, width: 58,
                                 mark: Number(modelData.unplanned) > 0 && !(Number(modelData.planned) > 0)}
                            ]
                        }
                    }
                }
            }
            RowLayout {
                Layout.fillWidth: true
                Layout.margins: 16
                visible: !f.result
                spacing: 4
                LabelText { objectName: 'plan-refusal'; text: f.refusal; color: Theme.danger; Layout.preferredWidth: 80 }
                Item { Layout.fillWidth: true }
                SButton { objectName: 'plan-draft'; visible: f.localDraft; text: 'Draft'; quiet: true; busy: f.saving; enabled: Number(f.coverage) >= 1; onClicked: f.draft() }
                SButton { objectName: 'plan-write'; text: 'Write plan'; primary: true; busy: f.saving; enabled: f.ready; onClicked: f.write() }
            }
        }
    }

    Component {
        id: importPane
        ColumnLayout {
            id: f
            property bool busy: false
            function run() {
                if (!bankField.text.trim() || busy) return;
                busy = true;
                var bank = bankField.text.trim();
                Backend.financeImport(bank, r => {
                    busy = false;
                    if (r.ok) pane.close();
                    else if (r.setup) Backend.openHearthPane('bank', {bank: bank});
                });
            }
            Component.onCompleted: { bankField.text = (pane.item || {}).bank || ''; bankField.forceActiveFocus(); Backend.loadBanks(); }
            spacing: 0
            ColumnLayout {
                Layout.fillWidth: true; Layout.margins: 24; spacing: 18
                LabelText { text: 'A statement file is chosen'; color: Theme.muted }
                Labelled { label: 'Bank'; Field { id: bankField; objectName: 'import-bank'; Layout.fillWidth: true; Accessible.name: 'Bank'; onAccepted: f.run() } }
                // The banks set up (T028): a pick that fills the field.
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
            RowLayout {
                Layout.fillWidth: true; Layout.margins: 16
                Item { Layout.fillWidth: true }
                SButton { objectName: 'import-run'; text: 'Import'; primary: true; busy: f.busy; enabled: !!bankField.text.trim(); onClicked: f.run() }
            }
        }
    }

    Component {
        id: linesPane
        ColumnLayout {
            id: f
            property var lines: []
            function load() { Backend.financeLines(pane.item.week, pane.item.category, l => f.lines = l.filter(x => x.category === pane.item.category)); }
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
                    x: 24; y: 24
                    width: linesScroll.width - 48
                    spacing: 4
                    Repeater {
                        model: f.lines
                        delegate: RowLayout {
                            id: line
                            required property var modelData
                            required property int index
                            Layout.fillWidth: true
                            Layout.preferredHeight: Theme.denseRowHeight
                            spacing: 12
                            LabelText { Layout.preferredWidth: 90; text: Backend.longDate(line.modelData.date); color: Theme.muted; font.pixelSize: 12 }
                            LabelText { Layout.fillWidth: true; text: line.modelData.merchant || ''; elide: Text.ElideRight }
                            LabelText { text: Backend.moneyText(line.modelData.amount, line.modelData.currency); font.family: Theme.mono }
                            Choice {
                                objectName: 'line-category-' + line.index
                                Layout.preferredWidth: 170
                                model: Backend.categoryWords.map(c => c.word)
                                currentIndex: Math.max(0, Backend.categoryWords.findIndex(c => c.key === line.modelData.category))
                                Accessible.name: 'Category'
                                onActivated: index => Backend.financeRecategorise(line.modelData.id, Backend.categoryWords[index].key, ok => f.load())
                            }
                        }
                    }
                }
            }
        }
    }

    Component {
        id: bankPane
        ColumnLayout {
            id: f
            property bool busy: false
            property var categories: [{bank: '', key: 'food:groceries'}]
            readonly property var formats: ['2026-10-05', '05/10/2026', '10/05/2026']
            readonly property var formatKeys: ['YYYY-MM-DD', 'DD/MM/YYYY', 'MM/DD/YYYY']
            readonly property bool ready: !!bankField.text.trim() && !!column(dateField, datePick) && !!column(descriptionField, descriptionPick)
                                         && !!column(amountField, amountPick) && !!column(currencyField, currencyPick)
            function save() {
                if (!ready || busy) return;
                var mapping = {
                    bank: bankField.text.trim(), date: column(dateField, datePick).trim(), date_format: formatKeys[formatChoice.currentIndex],
                    description: column(descriptionField, descriptionPick).trim(), amount: column(amountField, amountPick).trim(),
                    currency: column(currencyField, currencyPick).trim(),
                    debit_negative: negative.checked, categories: {}
                };
                if (categoryField.text.trim()) {
                    mapping.category = categoryField.text.trim();
                    categories.forEach(c => { if (c.bank.trim()) mapping.categories[c.bank.trim()] = c.key; });
                }
                busy = true;
                Backend.financeSaveBank(mapping, ok => {
                    busy = false;
                    if (ok) Backend.openHearthPane('import', {bank: mapping.bank});
                });
            }
            // The file's first line (T028): the four column fields become pickers.
            property var columns: []
            function column(field, picker) { return f.columns.length ? (picker.currentIndex >= 0 ? f.columns[picker.currentIndex] : '') : field.text; }
            function fill(m) {
                if (!m) return;
                dateField.text = m.date || ''; descriptionField.text = m.description || ''; amountField.text = m.amount || '';
                currencyField.text = m.currency || ''; categoryField.text = m.category || '';
                formatChoice.currentIndex = Math.max(0, formatKeys.indexOf(m.date_format));
                negative.checked = !!m.debit_negative;
                var c = Object.keys(m.categories || {}).map(k => ({bank: k, key: m.categories[k]}));
                if (c.length) categories = c;
                [[datePick, m.date], [descriptionPick, m.description], [amountPick, m.amount], [currencyPick, m.currency]]
                    .forEach(p => p[0].currentIndex = columns.indexOf(p[1]));
            }
            Component.onCompleted: {
                bankField.text = (pane.item || {}).bank || '';
                dateField.forceActiveFocus();
                Backend.statementHeader(c => {
                    f.columns = c;
                    // Each picker starts on the column its name suggests, never on
                    // whichever column happens to come first.
                    [[datePick, /date/i], [descriptionPick, /desc|detail|narrat|payee|merchant/i], [amountPick, /amount|value/i], [currencyPick, /curr/i]]
                        .forEach(p => p[0].currentIndex = c.findIndex(n => p[1].test(n)));
                    Backend.loadBanks(() => f.fill(Backend.financeBanks.find(b => b.bank === bankField.text.trim())));
                });
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
                    Labelled { label: 'Date column'; Field { id: dateField; visible: !f.columns.length; objectName: 'bank-date'; Layout.fillWidth: true; Accessible.name: 'Date column' }
                        Choice { id: datePick; objectName: 'bank-date-pick'; visible: f.columns.length > 0; Layout.fillWidth: true; model: f.columns; currentIndex: -1; Accessible.name: 'Date column' } }
                    Labelled { label: 'Description column'; Field { id: descriptionField; visible: !f.columns.length; objectName: 'bank-description'; Layout.fillWidth: true; Accessible.name: 'Description column' }
                        Choice { id: descriptionPick; objectName: 'bank-description-pick'; visible: f.columns.length > 0; Layout.fillWidth: true; model: f.columns; currentIndex: -1; Accessible.name: 'Description column' } }
                    Labelled { label: 'Amount column'; Field { id: amountField; visible: !f.columns.length; objectName: 'bank-amount'; Layout.fillWidth: true; Accessible.name: 'Amount column' }
                        Choice { id: amountPick; objectName: 'bank-amount-pick'; visible: f.columns.length > 0; Layout.fillWidth: true; model: f.columns; currentIndex: -1; Accessible.name: 'Amount column' } }
                    Labelled { label: 'Currency column'; Field { id: currencyField; visible: !f.columns.length; objectName: 'bank-currency'; Layout.fillWidth: true; Accessible.name: 'Currency column' }
                        Choice { id: currencyPick; objectName: 'bank-currency-pick'; visible: f.columns.length > 0; Layout.fillWidth: true; model: f.columns; currentIndex: -1; Accessible.name: 'Currency column' } }
                    Labelled { label: 'Date format'; Choice { id: formatChoice; objectName: 'bank-format'; Layout.preferredWidth: 200; model: f.formats; Accessible.name: 'Date format' } }
                    Labelled { label: 'Category column'; Field { id: categoryField; objectName: 'bank-category'; Layout.fillWidth: true; Accessible.name: 'Category column' } }
                    SCheckBox { id: negative; objectName: 'bank-negative'; text: 'Spending is written as a negative amount' }
                    Labelled {
                        visible: !!categoryField.text.trim()
                        label: 'Categories'
                        Repeater {
                            model: f.categories
                            delegate: RowLayout {
                                id: cat
                                required property var modelData
                                required property int index
                                spacing: 10
                                Field { objectName: 'bank-word-' + cat.index; Layout.preferredWidth: 160; text: cat.modelData.bank; Accessible.name: 'Bank category'; onTextEdited: f.categories[cat.index].bank = text }
                                Choice {
                                    objectName: 'bank-key-' + cat.index
                                    Layout.preferredWidth: 200
                                    model: Backend.categoryWords.map(c => c.word)
                                    currentIndex: Math.max(0, Backend.categoryWords.findIndex(c => c.key === cat.modelData.key))
                                    Accessible.name: 'Category'
                                    onActivated: index => f.categories[cat.index].key = Backend.categoryWords[index].key
                                }
                            }
                        }
                        SButton { objectName: 'bank-add-category'; text: 'Add category'; quiet: true; onClicked: f.categories = f.categories.concat([{bank: '', key: 'food:groceries'}]) }
                    }
                }
            }
            RowLayout {
                Layout.fillWidth: true; Layout.margins: 16
                Item { Layout.fillWidth: true }
                SButton { objectName: 'bank-save'; text: 'Save bank'; primary: true; busy: f.busy; enabled: f.ready; onClicked: f.save() }
            }
        }
    }
}
