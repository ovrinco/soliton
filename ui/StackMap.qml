pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root

    signal openModule(string moduleId)
    signal openPage(string page, string view)

    property int selectedIndex: 0
    property var moduleStates: ({})
    property string errorText: ''
    Component.onCompleted: if (Backend.available) refreshStates()
    onVisibleChanged: if (visible && Backend.available) refreshStates()
    Connections { target: Backend; function onAvailableChanged() { if (root.visible && Backend.available) root.refreshStates(); } }

    readonly property var layers: [
        {key:'core', label:'Core', note:'Conceptual commitments and the central machinery.'},
        {key:'inner', label:'Inner', note:'The control surface, memory and agent-tool roles.'},
        {key:'middle', label:'Middle', note:'Ideation, specification and execution roles.'},
        {key:'outer', label:'Outer', note:'Methods, understanding and bulk-data roles.'}
    ]

    // This list mirrors MODEL.json. Role and identity are separate because an
    // owned domain may change its implementation without changing its purpose.
    readonly property var domains: [
        {layer:'core', role:'Methodology', identity:'Citium', moduleId:'', page:'Strategy', view:'Overview', status:'Conceptual role', note:'The principles and methods used to frame, test and improve the work.'},
        {layer:'core', role:'Blueprint', identity:'Factory', moduleId:'', page:'Strategy', view:'Overview', status:'Conceptual role', note:'The structure of the work: what belongs where, and how its parts fit together.'},
        {layer:'core', role:'Machinery', identity:'STRATUM', moduleId:'', page:'Strategy', view:'Overview', status:'Conceptual role', note:'The machinery beneath the working environment.'},
        {layer:'inner', role:'Control interface', identity:'Soliton', moduleId:'', page:'Strategy', view:'Overview', status:'Native surface', note:'Your working surface for tasks, goals, records and files.'},
        {layer:'inner', role:'Memory system', identity:'Lorengine', moduleId:'', page:'Strategy', view:'Overview', status:'Standalone tool', note:'Find and revisit source material without losing its context.'},
        {layer:'inner', role:'Agent tools', identity:'Zeno', moduleId:'', page:'Strategy', view:'Overview', status:'Native console role', note:'Ask for help with the work. Review requests and permissions in the console.'},
        {layer:'middle', role:'Ideation workspace', identity:'Q-ROC', moduleId:'', page:'Strategy', view:'Overview', status:'Standalone tool', note:'Shape a project through its documents, branches and review gates.'},
        {layer:'middle', role:'Spec engine', identity:'Phod', moduleId:'', page:'Strategy', view:'Overview', status:'Standalone tool', note:'Write a specification, inspect diagnostics and review compiler artifacts.'},
        {layer:'middle', role:'Execution system', identity:'QREAM', moduleId:'', page:'Strategy', view:'Overview', status:'Standalone tool', note:'Inspect execution state and the artifacts returned by its owner.'},
        {layer:'outer', role:'SOPs / Skills', identity:'', moduleId:'', page:'Strategy', view:'Overview', status:'Conceptual role', note:'Reusable instructions for recurring work.'},
        {layer:'outer', role:'Global understanding', identity:'Zerde', moduleId:'', page:'Strategy', view:'Overview', status:'Not connected yet', note:'A role for connecting context across the working model.'},
        {layer:'outer', role:'Bulk data tools', identity:'Z-ETL', moduleId:'zetl', page:'Strategy', view:'Modules', status:'Bundled helper', note:'Prepare and transform bulk data.'}
    ]

    function selectedDomain() {
        return root.domains[root.selectedIndex] || root.domains[0];
    }

    function layerFor(key) {
        for (var i = 0; i < root.layers.length; ++i)
            if (root.layers[i].key === key)
                return root.layers[i];
        return root.layers[0];
    }

    function stateFor(domain) {
        if (!domain.moduleId)
            return domain.status;
        var state = root.moduleStates[domain.moduleId];
        if (!state)
            return (!Backend.available || root.errorText) ? 'Status unavailable' : domain.status;
        if (state.state === 'unavailable')
            return 'Unavailable · ' + (state.name || domain.identity);
        if (state.state === 'external')
            return 'External adapter';
        return state.state === 'available' ? domain.status : String(state.state || domain.status);
    }

    function choose(index) {
        if (index < 0 || index >= root.domains.length)
            return;
        root.selectedIndex = index;
    }

    function activate(domain) {
        if (!domain)
            return;
        if (domain.moduleId)
            root.openModule(domain.moduleId);
        else if(domain.identity==='Zeno')
            Backend.consoleOpen=true;
    }

    function refreshStates() {
        if (typeof Backend === 'undefined' || !Backend.send)
            return;
        Backend.send('modules-list', {}, function(value, ok, error) {
            if (!root)
                return;
            if (!ok) {
                root.errorText = error || 'Module capability status is unavailable.';
                return;
            }
            var next = {};
            (value.modules || []).forEach(function(module) { next[module.id] = module; });
            root.moduleStates = next;
            root.errorText = '';
        }, true);
    }

    // Coordinates refer to the recovered original diagram, not a new interpretation.
    readonly property var facePolygons: [
        [[530,589],[655,524],[781,589],[655,658]],
        [[530,589],[655,658],[655,793],[530,725]],
        [[655,658],[781,589],[781,725],[655,793]],
        [[410,525],[655,397],[901,525],[781,589],[655,524],[530,589]],
        [[410,525],[530,589],[530,725],[655,793],[655,919],[410,785]],
        [[781,589],[901,525],[901,785],[655,919],[655,793],[781,725]],
        [[270,452],[655,257],[1039,452],[901,525],[655,397],[410,525]],
        [[270,452],[410,525],[410,785],[655,919],[655,1063],[270,857]],
        [[901,525],[1039,452],[1039,857],[655,1063],[655,919],[901,785]],
        [[159,394],[655,136],[1150,394],[1039,452],[655,257],[270,452]],
        [[159,394],[270,452],[270,857],[655,1063],[655,1194],[159,943]],
        [[1039,452],[1150,394],[1150,943],[655,1194],[655,1063],[1039,857]]
    ]
    function faceAt(x,y) {
        for(var f=0;f<facePolygons.length;f++) {
            var polygon=facePolygons[f], inside=false;
            for(var i=0,j=polygon.length-1;i<polygon.length;j=i++) {
                var a=polygon[i], b=polygon[j];
                if(((a[1]>y)!==(b[1]>y)) && x<(b[0]-a[0])*(y-a[1])/(b[1]-a[1])+a[0]) inside=!inside;
            }
            if(inside)return f;
        }
        return -1;
    }

    SplitView {
        anchors.fill: parent
        orientation: root.width >= 700 ? Qt.Horizontal : Qt.Vertical
        handle: SSplitHandle { horizontal: root.width >= 700 }
        Item {
            SplitView.preferredWidth: root.width * .55
            SplitView.preferredHeight: Math.max(160, root.height * .46)
            SplitView.minimumHeight: Math.min(140, root.height * .32)
            Image {
                id: diagram
                objectName: 'stack-diagram'
                anchors.fill: parent; anchors.margins: 12
                source: '../assets/OVRIN_Working_Model_Geometric.svg'
                fillMode: Image.PreserveAspectFit; smooth: true
                Accessible.name: 'OVRIN working model; select an adjacent domain to open its workspace'
                MouseArea {
                    anchors.fill: parent
                    onClicked: mouse => {
                        var x=(mouse.x-(width-diagram.paintedWidth)/2)*1312/diagram.paintedWidth;
                        var y=(mouse.y-(height-diagram.paintedHeight)/2)*1199/diagram.paintedHeight;
                        var index=root.faceAt(x,y);
                        if(index>=0){root.choose(index);domainList.positionViewAtIndex(index,ListView.Contain);}
                    }
                    onDoubleClicked: root.activate(root.selectedDomain())
                }
            }
        }
        ColumnLayout {
            SplitView.fillWidth: true; SplitView.fillHeight: true
            SplitView.minimumWidth: 240; SplitView.minimumHeight: Math.min(220, root.height * .62)
            spacing: 12
        ListView {
            id: domainList
            SCollectionWheel { flickable: domainList }
            objectName: 'stack-domain-list'
            Layout.fillWidth: true; Layout.fillHeight: true; Layout.minimumWidth: 0; Layout.minimumHeight: Math.min(80,root.height * .20)
            clip: true; spacing: 1; model: root.domains
            delegate: ItemDelegate {
                id: domainRow
                required property var modelData
                required property int index
                width: domainList.width; height: Theme.twoLineRowHeight
                topPadding: 3; bottomPadding: 3
                highlighted: root.selectedIndex === domainRow.index
                onClicked: root.choose(domainRow.index)
                onDoubleClicked: root.activate(domainRow.modelData)
                background: Rectangle { color: domainRow.highlighted ? Theme.selection : domainRow.hovered ? Theme.hover : 'transparent' }
                contentItem: RowLayout {
                    ColumnLayout {
                        Layout.fillWidth: true; spacing: 1
                        LabelText { text: domainRow.modelData.role; Layout.fillWidth:true; elide:Text.ElideRight }
                        LabelText { text: (domainRow.modelData.identity ? domainRow.modelData.identity + ' · ' : '') + root.layerFor(domainRow.modelData.layer).label; color:Theme.muted; font.pixelSize: 12; Layout.fillWidth:true; elide:Text.ElideRight }
                    }
                    SButton { text:'Open'; quiet:true; visible:!!domainRow.modelData.moduleId || domainRow.modelData.identity==='Zeno'; onClicked:root.activate(domainRow.modelData) }
                }
            }
            ScrollBar.vertical: SScrollBar {}
        }
            Item {
                Layout.fillWidth: true
                Layout.preferredHeight: Math.min(190,Math.max(72,root.height*0.26))
                Flickable {
                    anchors.fill: parent; clip: true
                    contentWidth: width; contentHeight: domainInfo.implicitHeight + 12
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: SScrollBar { }
                    ColumnLayout {
                        id: domainInfo; width: parent.width; spacing: 8
                        LabelText { Layout.fillWidth:true; text:root.selectedDomain().role; font.pixelSize:18; font.weight:Font.DemiBold }
                        LabelText { Layout.fillWidth:true; text:root.selectedDomain().note; wrapMode:Text.Wrap; elide:Text.ElideNone; color:Theme.muted }
                        LabelText { Layout.fillWidth:true; text:root.stateFor(root.selectedDomain()); wrapMode:Text.Wrap; color:Theme.muted; font.pixelSize:12 }
                    }
                }
            }
        }
    }
}
