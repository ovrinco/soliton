pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Hearth, Kitchen: what goes off soon, the stock with its pictures and weights,
// the food bought in Finance and not yet in stock, the equipment and the
// cook's standing constraints, all read from the engine's kitchen-* calls.
FocusScope {
    id: view
    objectName: 'kitchen-view'

    readonly property string tab: Backend.kitchenTab
    readonly property var stock: Backend.kitchenStock
    readonly property bool stockEmpty: Backend.kitchenLoaded && stock.length === 0 && purchases.length === 0
    readonly property bool equipmentEmpty: Backend.kitchenLoaded && Backend.kitchenEquipment.length === 0
    readonly property bool canPlan: Backend.kitchenLoaded && (stock.length > 0 || Backend.kitchenEquipment.length > 0)
    readonly property var fixedSubjects: ['portions_per_week', 'session_hours', 'diet']
    // What the plan must leave out and what it must use: always two rows,
    // blank until set, ahead of any other constraint.
    readonly property var namedSubjects: ['exclude', 'required']
    readonly property var named: namedSubjects.map(s => profileEntry(s) || ({subject: s, value: ''}))
    readonly property var others: Backend.kitchenProfile.filter(e => fixedSubjects.indexOf(e.subject) < 0 && namedSubjects.indexOf(e.subject) < 0)
    readonly property var spans: [3, 7, 14, 30]
    // The food lines Finance holds that no lot names (kitchen-from-purchases),
    // read again whenever the stock is.
    property var purchases: []
    function loadPurchases() {
        Backend.send('kitchen-from-purchases', {}, (r, ok) => { view.purchases = ok ? (r.lines || []) : []; }, true);
    }
    Connections {
        target: Backend
        function onKitchenStockChanged() { view.loadPurchases(); }
    }
    Component.onCompleted: loadPurchases()
    // A lot going off within the look-ahead is marked.
    readonly property string spanEnd: {
        var p = String(Backend.today || '').split('-');
        if (p.length !== 3) return '';
        var d = new Date(Number(p[0]), Number(p[1]) - 1, Number(p[2]) + Backend.kitchenSpan);
        return d.getFullYear() + '-' + String(d.getMonth() + 1).padStart(2, '0') + '-' + String(d.getDate()).padStart(2, '0');
    }
    function goesOffSoon(iso) { return !!iso && !!spanEnd && String(iso) <= spanEnd; }
    function pictureOf(subject, expires) {
        var l = stock.find(x => x.subject === subject && (x.expires_on || '') === (expires || '') && !!x.picture);
        return l ? l.picture : '';
    }
    function subjectWord(subject) {
        var words = {exclude: 'Exclude', required: 'Required'};
        if (words[subject]) return words[subject];
        var text = String(subject || '').replace(/[^A-Za-z0-9]+/g, ' ').trim();
        return text.charAt(0).toUpperCase() + text.slice(1);
    }

    function profileEntry(subject) { return Backend.kitchenProfile.find(e => e.subject === subject) || null; }
    function entryText(e) {
        if (!e) return '';
        if (e.value !== null && e.value !== undefined && e.value !== '') return e.value;
        return Backend.amountText(e.quantity, e.unit);
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            spacing: 4
            Item { Layout.fillWidth: true }
            STabBar {
                Layout.preferredWidth: 3 * 112
                STabButton { objectName: 'kitchen-tab-stock'; Layout.preferredWidth: 112; text: 'Stock'; active: view.tab === 'Stock'; onClicked: Backend.setKitchenTab('Stock') }
                STabButton { objectName: 'kitchen-tab-equipment'; Layout.preferredWidth: 112; text: 'Equipment'; active: view.tab === 'Equipment'; onClicked: Backend.setKitchenTab('Equipment') }
                STabButton { objectName: 'kitchen-tab-profile'; Layout.preferredWidth: 112; text: 'Profile'; active: view.tab === 'Profile'; onClicked: Backend.setKitchenTab('Profile') }
            }
        }

        // Nothing to show: one muted line and the one primary button, quiet
        // while private mode is on.
        ColumnLayout {
            visible: (view.tab === 'Stock' && view.stockEmpty) || (view.tab === 'Equipment' && view.equipmentEmpty)
            Layout.alignment: Qt.AlignLeft
            spacing: 16
            SButton {
                objectName: 'kitchen-empty-add'
                Layout.alignment: Qt.AlignLeft
                text: view.tab === 'Stock' ? 'Add to stock' : 'Add equipment'
                primary: !Backend.privateMode; quiet: Backend.privateMode
                onClicked: Backend.kitchenAdd()
            }
        }

        Flickable {
            id: scroll
            visible: Backend.kitchenLoaded && !(view.tab === 'Stock' && view.stockEmpty) && !(view.tab === 'Equipment' && view.equipmentEmpty)
            Layout.fillWidth: true; Layout.fillHeight: true
            clip: true
            contentWidth: width
            contentHeight: body.implicitHeight + 16
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: SScrollBar { }

            ColumnLayout {
                id: body
                width: scroll.width - 12
                spacing: 28

                // Stock tab: what goes off soon, then the whole stock.
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: view.tab === 'Stock'
                    spacing: 6
                    RowLayout {
                        Layout.fillWidth: true
                        LabelText { Layout.fillWidth: true; text: 'Goes off soon'; font.pixelSize: Theme.sectionTitleSize; font.weight: Font.Medium }
                        SButton {
                            objectName: 'kitchen-span'
                            quiet: true
                            text: Backend.kitchenSpan + ' days'
                            onClicked: Backend.setKitchenSpan(view.spans[(view.spans.indexOf(Backend.kitchenSpan) + 1) % view.spans.length])
                        }
                    }
                    Repeater {
                        model: Backend.kitchenGoesOff
                        delegate: HearthRow {
                            required property var modelData
                            Layout.fillWidth: true
                            opens: false
                            pictureSlot: true
                            picture: view.pictureOf(modelData.subject, modelData.expires_on)
                            name: modelData.subject
                            cells: [
                                {text: Backend.amountText(modelData.on_hand, modelData.unit), mono: true, right: true, width: 110},
                                {text: Backend.longDate(modelData.expires_on), width: 150, mark: true},
                                {text: Number(modelData.planned) > 0 ? Backend.amountText(modelData.planned, modelData.unit) + ' planned' : '', muted: true, small: true, width: 160}
                            ]
                        }
                    }
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: view.tab === 'Stock'
                    spacing: 6
                    LabelText { text: 'Stock'; font.pixelSize: Theme.sectionTitleSize; font.weight: Font.Medium }
                    Repeater {
                        model: view.stock
                        delegate: HearthRow {
                            required property var modelData
                            Layout.fillWidth: true
                            pictureSlot: true
                            picture: modelData.picture || ''
                            name: modelData.subject
                            cells: [
                                {text: Backend.amountText(modelData.quantity, modelData.unit), mono: true, right: true, width: 110},
                                {text: modelData.expires_on ? Backend.longDate(modelData.expires_on) : '', width: 150, mark: view.goesOffSoon(modelData.expires_on)},
                                {text: modelData.location || '', muted: true, width: 160}
                            ]
                            actions: [
                                {text: 'Edit', run: () => Backend.openHearthPane('lot', modelData)},
                                {text: 'Used up', run: () => Backend.kitchenUsedUp(modelData.id)}
                            ]
                            onActivated: Backend.openHearthPane('lot', modelData)
                        }
                    }
                }

                // Food bought in Finance that no lot names yet: each line once,
                // to stock through its menu; a line in stock leaves for good.
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: view.tab === 'Stock' && view.purchases.length > 0
                    spacing: 6
                    LabelText { text: 'From purchases'; font.pixelSize: Theme.sectionTitleSize; font.weight: Font.Medium }
                    Repeater {
                        model: view.purchases
                        delegate: HearthRow {
                            required property var modelData
                            Layout.fillWidth: true
                            opens: false
                            pictureSlot: true
                            name: modelData.subject
                            cells: [
                                {text: Backend.moneyText(modelData.amount, modelData.currency), mono: true, right: true, width: 110},
                                {text: Backend.longDate(modelData.day), width: 150},
                                {text: Backend.categoryWord(modelData.category), muted: true, width: 160}
                            ]
                            actions: [{text: 'To stock', run: () => Backend.openHearthPane('newLot', {purchase: modelData})}]
                        }
                    }
                }

                // Equipment tab.
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: view.tab === 'Equipment'
                    spacing: 6
                    LabelText { text: 'Equipment'; font.pixelSize: Theme.sectionTitleSize; font.weight: Font.Medium }
                    Repeater {
                        model: Backend.kitchenEquipment
                        delegate: HearthRow {
                            required property var modelData
                            Layout.fillWidth: true
                            name: modelData.name
                            cells: [
                                {text: Backend.amountText(modelData.count, ''), mono: true, right: true, width: 60},
                                {text: modelData.capacity || '', muted: true, width: 120}
                            ]
                            actions: [
                                {text: 'Edit', run: () => Backend.openHearthPane('equipment', modelData)},
                                {text: 'Remove', run: () => Backend.kitchenSetEquipment(modelData.name, 0, modelData.capacity)}
                            ]
                            onActivated: Backend.openHearthPane('equipment', modelData)
                        }
                    }
                }

                // Profile tab: three fixed fields, then the other constraints.
                ColumnLayout {
                    id: profile
                    Layout.fillWidth: true
                    visible: view.tab === 'Profile'
                    spacing: 18
                    property string portions: ''
                    property string hours: ''
                    property string diet: ''
                    property bool saving: false
                    readonly property string portionsWas: view.entryText(view.profileEntry('portions_per_week')).replace(/ .*$/, '')
                    readonly property string hoursWas: view.entryText(view.profileEntry('session_hours')).replace(/ .*$/, '')
                    readonly property string dietWas: view.entryText(view.profileEntry('diet'))
                    readonly property bool dirty: portions !== portionsWas || hours !== hoursWas || diet !== dietWas
                    function reset() {
                        portions = portionsWas; hours = hoursWas; diet = dietWas;
                        portionsField.text = portions; hoursField.text = hours; dietField.text = diet;
                    }
                    function save() {
                        var entries = [];
                        if (portions !== portionsWas && portions !== '') entries.push({subject: 'portions_per_week', quantity: Number(portions)});
                        if (hours !== hoursWas && hours !== '') entries.push({subject: 'session_hours', quantity: Number(hours), unit: 'h'});
                        if (diet !== dietWas && diet.trim() !== '') entries.push({subject: 'diet', value: diet.trim()});
                        saving = true;
                        Backend.kitchenSetProfile(entries, ok => { saving = false; });
                    }
                    Connections {
                        target: Backend
                        function onKitchenProfileChanged() { if (!profile.saving) profile.reset(); }
                    }
                    Component.onCompleted: reset()
                    onSavingChanged: if (!saving) reset()

                    component Label: LabelText { font.pixelSize: 12; color: Theme.muted }
                    ColumnLayout {
                        spacing: 6
                        Label { text: 'Portions a week' }
                        Field {
                            id: portionsField
                            objectName: 'profile-portions'
                            Layout.preferredWidth: 120
                            validator: IntValidator { bottom: 0; top: 9999 }
                            Accessible.name: 'Portions a week'
                            onTextEdited: profile.portions = text
                        }
                    }
                    ColumnLayout {
                        spacing: 6
                        Label { text: 'Session length' }
                        RowLayout {
                            spacing: 10
                            Field {
                                id: hoursField
                                objectName: 'profile-hours'
                                Layout.preferredWidth: 120
                                validator: DoubleValidator { bottom: 0; top: 24; notation: DoubleValidator.StandardNotation }
                                Accessible.name: 'Session length'
                                onTextEdited: profile.hours = text
                            }
                            LabelText { text: 'hours'; color: Theme.muted }
                        }
                    }
                    ColumnLayout {
                        spacing: 6
                        Label { text: 'Diet' }
                        Field {
                            id: dietField
                            objectName: 'profile-diet'
                            Layout.preferredWidth: 360
                            Accessible.name: 'Diet'
                            onTextEdited: profile.diet = text
                        }
                    }
                    RowLayout {
                        visible: profile.dirty
                        spacing: 4
                        SButton { text: 'Cancel'; quiet: true; onClicked: profile.reset() }
                        SButton { objectName: 'profile-save'; text: 'Save'; quiet: true; busy: profile.saving; onClicked: profile.save() }
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 6
                        LabelText { text: 'Other constraints'; font.pixelSize: Theme.sectionTitleSize; font.weight: Font.Medium }
                        Repeater {
                            model: view.named
                            delegate: HearthRow {
                                required property var modelData
                                Layout.fillWidth: true
                                name: view.subjectWord(modelData.subject)
                                cells: [{text: view.entryText(modelData), width: 240}]
                                actions: modelData.id
                                    ? [{text: 'Edit', run: () => Backend.openHearthPane('constraint', modelData)},
                                       {text: 'Remove', run: () => Backend.kitchenRemoveProfile(modelData.subject)}]
                                    : [{text: 'Edit', run: () => Backend.openHearthPane('constraint', modelData)}]
                                onActivated: Backend.openHearthPane('constraint', modelData)
                            }
                        }
                        Repeater {
                            model: view.others
                            delegate: HearthRow {
                                required property var modelData
                                Layout.fillWidth: true
                                name: view.subjectWord(modelData.subject)
                                cells: [{text: view.entryText(modelData), width: 240}]
                                actions: [{text: 'Edit', run: () => Backend.openHearthPane('constraint', modelData)},
                                          {text: 'Remove', run: () => Backend.kitchenRemoveProfile(modelData.subject)}]
                                onActivated: Backend.openHearthPane('constraint', modelData)
                            }
                        }
                    }
                }
            }
        }
        Item { Layout.fillHeight: true; visible: !scroll.visible }

        // A plan can be drawn on the equipment alone, so an empty stock does
        // not hide it; the empty state keeps its own add button.
        RowLayout {
            Layout.fillWidth: true
            visible: scroll.visible || (view.canPlan && view.tab !== 'Profile')
            spacing: 4
            Item { Layout.fillWidth: true }
            SButton { objectName: 'kitchen-plan'; visible: view.canPlan && view.tab !== 'Profile'; text: 'Plan a batch'; quiet: true; onClicked: Backend.openHearthPane('plan') }
            SButton {
                objectName: 'kitchen-add'
                visible: scroll.visible
                text: view.tab === 'Equipment' ? 'Add equipment' : view.tab === 'Profile' ? 'Add constraint' : 'Add to stock'
                quiet: true
                onClicked: Backend.kitchenAdd()
            }
        }
    }
}
