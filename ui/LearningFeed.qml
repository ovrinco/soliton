import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Workplane.js" as Workplane

Item {
    id: root
    signal archiveRequested(string sourceId)
    // Opens a YouTube Intel view: 'Suggestions' or 'To analyze'.
    signal videosRequested(string view)
    property var items: []
    property var providers: []
    property var rules: []
    property var jobs: []
    property var selected: null
    // Feed lists articles; YouTube suggestions have their own view in YouTube Intel.
    property string channel: 'news'
    property string query: ''
    property string errorText: ''
    property bool loading: false
    property bool collecting: false
    property bool cycling: false
    property string collectionId: ''
    property string collectionJobId: ''
    property string collectionTitle: ''
    property string focusGoal: ''
    property string viewMode: 'Feed'
    property var excludedUndo: null
    property string focusCategory: ''
    property bool focused: false
    property var policies: ({})
    property int policyEpoch: 0
    property bool policyReady: false
    property bool alive: true
    function localRule(item, kind) { return rules.find(r=>r.matcher==='url' && r.pattern===item.url && r.kind===kind && r.enabled && r.scope===(focusGoal?'goal':'global') && (!focusGoal||r.goal_id===focusGoal)); }
    function matchesFor(item) {
        let text=(item.title+' '+item.summary+' '+item.publisher).toLowerCase();
        let found=[];
        (Backend.goals||[]).filter(g=>(!focusGoal||g.id===focusGoal)&&(!focusCategory||g.category_id===focusCategory)).forEach(goal=>{
            let terms=String(goal.title||'').toLowerCase().split(/[^a-z0-9]+/).filter(w=>w.length>=4 && ['with','from','this','that','have','make','your','more','work'].indexOf(w)<0);
            let matches=terms.filter(w=>new RegExp('\\b'+w+'\\b','i').test(text));
            if(matches.length) found.push({goal:goal.title,terms:matches});
        });
        return found;
    }
    function visibleItems() {
        let words=query.toLowerCase().trim();
        let list=items.filter(item=>{
            let policy=policies[item.id];
            if(!policy)return false;
            let excluded=!!policy.excluded, saved=(policy.matched||[]).some(r=>r.kind==='save');
            if(excluded)return false;
            if(viewMode==='Saved' && !saved)return false;
            if(channel!=='All' && item.provider!==channel)return false;
            if(words && (item.title+' '+item.publisher+' '+item.summary).toLowerCase().indexOf(words)<0)return false;
            if(focused && !saved && matchesFor(item).length===0)return false;
            return true;
        });
        if(focused) list.sort((a,b)=>matchesFor(b).length-matchesFor(a).length);
        return list;
    }
    function evaluatePolicies() {
        let epoch=++policyEpoch;
        policyReady=false;
        function page(offset,previous) {
            Backend.send('source-policy-evaluate-batch',{items:items.slice(offset,offset+200).map(i=>({source_id:i.id,url:i.url,title:i.title,goal_id:focusGoal}))},(r,ok,e)=>{
                if(!root||!root.alive||epoch!==root.policyEpoch)return;
                if(!ok){errorText=e;return;}
                (r.items||[]).forEach(p=>previous[p.source_id]=p);
                if(offset+200<items.length)page(offset+200,previous);
                else {policies=previous;policyReady=true;
                    let visible=root.visibleItems();
                    if(root.selected && !visible.some(i=>i.id===root.selected.id))root.selected=visible.length?visible[0]:null;
                    else if(!root.selected && visible.length)root.selected=visible[0];
                }
            });
        }
        page(0,{});
    }
    onFocusGoalChanged:evaluatePolicies()
    function refresh() {
        loading=true;
        Backend.send('collection-discover',{},(r,ok,e)=>{if(!root||!root.alive)return;loading=false;if(ok){items=r.items||[];providers=r.providers||[];if(!selected&&items.length)selected=items[0];evaluatePolicies();}else errorText=e;});
        reloadRules(); pollJobs(); Backend.refreshModelChosen();
    }
    function reloadRules(offset, accumulated) {
        let start=offset||0, previous=accumulated||[];
        Backend.send('source-policy-list',{limit:100,offset:start},(r,ok,e)=>{if(!root||!root.alive||!ok)return;let batch=previous.concat(r.items||[]);if(batch.length<(r.total||0))reloadRules(start+100,batch);else {rules=batch;evaluatePolicies();}});
    }
    function toggleRule(kind) {
        if(!selected)return;
        let prior=localRule(selected,kind);
        let payload=prior?{id:prior.id,expected_revision:prior.revision}:{scope:focusGoal?'goal':'global',goal_id:focusGoal,kind:kind,matcher:'url',pattern:selected.url};
        let item=selected;
        Backend.send(prior?'source-policy-remove':'source-policy-set',payload,(r,ok,e)=>{if(!root||!root.alive)return;if(ok){
            if(kind==='exclude')root.excludedUndo=prior?null:{url:item.url,title:item.title};
            reloadRules();
        }else errorText=e;});
    }
    function undoExclusion() {
        if(!excludedUndo)return;
        let rule=rules.find(r=>r.matcher==='url' && r.pattern===excludedUndo.url && r.kind==='exclude' && r.enabled);
        if(!rule){excludedUndo=null;return;}
        Backend.send('source-policy-remove',{id:rule.id,expected_revision:rule.revision},(r,ok,e)=>{
            if(!root||!root.alive)return;
            if(ok){excludedUndo=null;reloadRules();}else errorText=e;
        });
    }
    function removeRule(rule) {
        Backend.send('source-policy-remove',{id:rule.id,expected_revision:rule.revision},(r,ok,e)=>{
            if(!root||!root.alive)return;
            if(ok){if(excludedUndo && excludedUndo.url===rule.pattern)excludedUndo=null;reloadRules();}
            else errorText=e;
        });
    }
    function collect() {
        if(!selected||collecting)return;
        collecting=true;errorText='';collectionTitle=selected.title;
        Backend.send('collection-add',{type:selected.kind,url:selected.url,cadence_hours:24,auto_briefing:Backend.prefs.localModel===true,goal_ids:root.focusGoal?[root.focusGoal]:[]},(r,ok,e)=>{if(!root||!root.alive)return;collecting=false;if(ok){collectionId=r.id;collectionJobId=r.job_id||'';pollJobs();}else errorText=e;});
    }
    function pollJobs() {
        if(!collectionJobId)return;
        Backend.send('collection-job-status',{id:collectionJobId},(r,ok,e)=>{if(!root||!root.alive)return;if(ok)jobs=[r];else errorText=e;});
    }
    function currentJob() { return jobs.find(j=>j.id===collectionJobId)||null; }
    function age(timestamp) { let days=Math.floor((Date.now()/1000-Number(timestamp||0))/86400);return days<=0?'today':days===1?'yesterday':days+' days ago'; }
    function providerCount(id) { let p=providers.find(v=>v.id===id); return p&&p.available?Number(p.count||0):0; }
    // A curated short note: a description is cut to a few lines, never a body.
    function shortNote(value, limit) { let text=Workplane.previewLine(value); return text.length>limit?text.slice(0,limit-1).replace(/\s+\S*$/,'')+'…':text; }
    function publishedLabel(value) {
        if(!value)return '';
        if(typeof value==='number')return Qt.formatDate(new Date(value),'MMM d, yyyy');
        return String(value);
    }
    Component.onCompleted:refresh()
    Component.onDestruction:alive=false
    Timer { interval:2500;repeat:true;running:root.visible && !!root.collectionId;onTriggered:root.pollJobs() }
    Timer { interval:10000;repeat:true;running:root.cycling&&root.visible&&!Backend.prefs.reducedMotion;onTriggered:{let list=root.visibleItems();if(list.length){let n=list.findIndex(i=>root.selected&&i.id===root.selected.id);root.selected=list[(n+1)%list.length];}} }
    component FeedLine: AbstractButton {
        id: line
        property string heading: ''
        property string note: ''
        property string source: ''
        property string date: ''
        property bool selected: false
        implicitHeight: 26
        padding: 0
        hoverEnabled: true
        activeFocusOnTab: true
        Accessible.role: Accessible.ListItem
        Accessible.name: heading + (source ? '. ' + source : '') + (date ? '. ' + date : '')
        Accessible.selected: selected
        background: Rectangle {
            color: line.selected ? Theme.wash : line.down ? Theme.selection : line.hovered ? Theme.hover : 'transparent'
            border.width: line.visualFocus ? 2 : 0
            border.color: Theme.accent
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line }
        }
        contentItem: RowLayout {
            spacing: 12
            LabelText { Layout.leftMargin: 8; Layout.fillWidth: true; text: Workplane.previewLine(line.heading) + (line.note ? '  <font color="' + Theme.muted + '">' + line.note.replace(/&/g, '&amp;').replace(/</g, '&lt;') + '</font>' : ''); textFormat: Text.StyledText; elide: Text.ElideRight; maximumLineCount: 1; font.pixelSize: 13 }
            LabelText { Layout.preferredWidth: 170; text: line.source; elide: Text.ElideRight; color: Theme.muted; font.pixelSize: 12 }
            LabelText { Layout.preferredWidth: 96; Layout.rightMargin: 8; text: line.date; elide: Text.ElideRight; color: Theme.muted; font.pixelSize: 12 }
        }
    }
    ColumnLayout {
        anchors.fill:parent; spacing:8
        RowLayout {
            Layout.fillWidth:true; Layout.preferredHeight:Theme.controlHeight; spacing:8
            LabelText { text:'Feed';font.weight:Font.DemiBold }
            SSearchField { Layout.fillWidth:true; frameless:true; scopeName:'articles, sites and topics'; text:root.query; onTextEdited:root.query=text }
            SButton { visible:!!root.excludedUndo;text:'Undo exclusion';quiet:true;onClicked:root.undoExclusion() }
            SButton { id:filterButton;objectName:'learning-filter-trigger';iconName:'filter'; quiet:true; accessibleLabel:'Feed filters and sources'; onClicked:discoveryOptions.open() }
        }
        // The video sources open from the first screen of the Learning Center.
        RowLayout {
            Layout.fillWidth:true; spacing:8
            LabelText { text:'Videos'; color:Theme.muted; font.pixelSize:12 }
            SButton { objectName:'learning-youtube-suggestions'; text:'YouTube suggestions · '+root.providerCount('youtube'); quiet:true; enabled:root.providerCount('youtube')>0; onClicked:root.videosRequested('Suggestions') }
            SButton { objectName:'learning-youtube-add'; text:'Add a YouTube video'; quiet:true; onClicked:root.videosRequested('To analyze') }
            Item { Layout.fillWidth:true }
            LabelText { text:feedList.count+(feedList.count===1?' article':' articles'); color:Theme.muted; font.pixelSize:12 }
        }
        SplitView {
            Layout.fillWidth:true;Layout.fillHeight:true;orientation:root.width < 640 ? Qt.Vertical : Qt.Horizontal
            handle: SSplitHandle { horizontal: root.width >= 640 }
            ColumnLayout {
                SplitView.preferredHeight:root.height*.5
                SplitView.preferredWidth:Math.max(420,root.width*.58)
                SplitView.minimumWidth:300;SplitView.minimumHeight:110
                spacing:0
                RowLayout {
                    Layout.fillWidth:true; Layout.preferredHeight:24; spacing:12
                    LabelText { Layout.leftMargin:8; Layout.fillWidth:true; text:'Title'; color:Theme.muted; font.pixelSize:12 }
                    LabelText { Layout.preferredWidth:170; text:'Source'; color:Theme.muted; font.pixelSize:12 }
                    LabelText { Layout.preferredWidth:96; Layout.rightMargin:8; text:'Published'; color:Theme.muted; font.pixelSize:12 }
                }
                Rectangle { Layout.fillWidth:true; implicitHeight:1; color:Theme.line }
            ListView {
                id:feedList;objectName:'learning-feed-list';Layout.fillWidth:true;Layout.fillHeight:true;Layout.minimumHeight:0; RetainedScroll { flickable: feedList; positionKey: 'Learning/feed' }clip:true;spacing:0
                model:root.visibleItems();ScrollBar.vertical:SScrollBar {}
                delegate:FeedLine {
                    required property var modelData
                    width:feedList.width
                    heading:modelData.title
                    note:root.shortNote(modelData.summary, 140)
                    source:modelData.publisher || ''
                    date:root.publishedLabel(modelData.published)
                    selected:root.selected && root.selected.id === modelData.id
                    onClicked:{ root.selected=modelData; root.cycling=false; }
                }
                LabelText {anchors.centerIn:parent;visible:feedList.count===0;text:root.policyReady?'Nothing in the feed matches these filters.':'Loading source preferences…';color:Theme.muted}
            }
            }
            Rectangle {
                SplitView.fillWidth:true;SplitView.minimumWidth:280;color:Theme.base;radius:0;border.color:'transparent'
                ScrollView {
                    id:readerScroll
                    RetainedScroll { flickable: readerScroll.contentItem; positionKey: 'Learning/source/' + (root.selected ? root.selected.id : '') }
                    anchors.fill:parent;anchors.margins:20;contentWidth:availableWidth;clip:true
                    ColumnLayout {
                        width:readerScroll.availableWidth;spacing:16
                        LabelText {text:root.selected?root.selected.publisher:'Select an article';color:Theme.accent;Layout.fillWidth:true;font.pixelSize:12}
                        LabelText {text:root.selected?root.selected.title:'Start with the ideas already arriving in your existing sources.';font.pixelSize:26;font.weight:Font.DemiBold;Layout.fillWidth:true;wrapMode:Text.Wrap}
                        LabelText {text:root.selected?root.shortNote(root.selected.summary, 280):'';visible:!!text;Layout.fillWidth:true;wrapMode:Text.Wrap;color:Theme.text;font.pixelSize:15}
                        Rectangle {Layout.fillWidth:true;implicitHeight:1;color:Theme.line}
                        Flow {Layout.fillWidth:true;Layout.preferredHeight:implicitHeight;spacing:8
                            SButton {
                busy: root.collecting
text:root.collecting?'Queuing…':(root.selected&&root.selected.kind==='youtube-video'?'Collect':'Collect')+(Backend.prefs.localModel===true&&Backend.modelChosen?' & brief ':' ')+(root.selected&&root.selected.kind==='youtube-video'?'video':'article');primary:true;enabled:!!root.selected&&!root.collecting;onClicked:root.collect()}
                            SButton {text:'Open original';enabled:!!root.selected;onClicked:Qt.openUrlExternally(root.selected.url)}
                        }
                        LabelText {text:root.selected?root.matchesFor(root.selected).map(m=>m.goal+' · '+m.terms.join(', ')).join('\n'):'';visible:!!text;Layout.fillWidth:true;wrapMode:Text.Wrap;color:Theme.accent;font.pixelSize: 12}
                        RowLayout {
                            SButton {text:root.selected&&root.localRule(root.selected,'save')?'Saved · undo':'Save for later';enabled:!!root.selected;onClicked:root.toggleRule('save')}
                            SButton {text:root.selected&&root.localRule(root.selected,'exclude')?'Excluded · undo':'Exclude';enabled:!!root.selected;onClicked:root.toggleRule('exclude')}
                        }
                        Rectangle {
                            visible:!!root.collectionId;Layout.fillWidth:true;implicitHeight:jobBody.implicitHeight+24;color:Theme.base;border.color:Theme.line;radius: Theme.cornerRadius
                            ColumnLayout {id:jobBody;anchors.fill:parent;anchors.margins:12;spacing:8
                                LabelText {text:root.collectionTitle;Layout.fillWidth:true;wrapMode:Text.Wrap;font.weight:Font.DemiBold}
                                LabelText {property var job:root.currentJob();text:job?(job.acked?(job.briefing&&job.briefing.error?'Archived · briefing needs attention: '+job.briefing.error:job.briefing?(job.briefing.state==='waiting'||!Backend.modelChosen?'Archived · awaiting your model':'Archived · local briefing queued'):(Backend.prefs.localModel===true?(Backend.modelChosen?'Archived · preparing briefing':'Archived · awaiting your model'):'Archived')):job.state==='completed'?'Collected · archiving evidence':job.state)+(job.error?' · '+job.error:''):'Queued for collection';Layout.fillWidth:true;wrapMode:Text.Wrap;color:job&&job.state==='failed'?Theme.danger:Theme.accent}
                                SButton {property var job:root.currentJob();visible:!!job&&!!job.acked;text:Backend.prefs.localModel===true&&Backend.modelChosen?'Read & generate briefing':'Read';primary:true;onClicked:root.archiveRequested((job.history_ids||[])[0]||'')}
                            }
                        }
                        LabelText {visible:!!root.errorText;text:root.errorText;Layout.fillWidth:true;wrapMode:Text.Wrap;color:Theme.danger}
                    }
                }
            }
        }
    }
    SPopup {
        id:discoveryOptions; parent:Overlay.overlay; trigger:filterButton; alignRight:true
        width:Math.min(420,parent.width-32); height:Math.min(managePreferences.expanded?600:420,parent.height-32); padding:16
        modal:false; focus:true
        contentItem:ScrollView { id:discoveryScroll;
            contentWidth:availableWidth; clip:true
            ColumnLayout {
                width:discoveryScroll.availableWidth; spacing:14
                SectionLabel {text:'Feed filters'}
                SCheckBox {text:'Saved articles only';checked:root.viewMode==='Saved';onToggled:root.viewMode=checked?'Saved':'Feed'}
                SCheckBox {text:'Match selected goals';checked:root.focused;onToggled:root.focused=checked;Accessible.description:'Uses title and summary terms and your saved sources; the full briefing determines relevance.'}
                Choice {Layout.fillWidth:true;model:[{id:'',title:'All categories'}].concat(Backend.categories||[]);textRole:'title';currentIndex:Math.max(0,model.findIndex(r=>r.id===root.focusCategory));onActivated:{root.focusCategory=model[currentIndex].id;root.focusGoal='';}Accessible.name:'Feed category'}
                Choice {Layout.fillWidth:true;model:[{id:'',title:'All goals'}].concat((Backend.goals||[]).filter(g=>!root.focusCategory||g.category_id===root.focusCategory));textRole:'title';currentIndex:Math.max(0,model.findIndex(r=>r.id===root.focusGoal));onActivated:root.focusGoal=model[currentIndex].id;Accessible.name:'Feed goal'}
                SDisclosure {
                    Layout.fillWidth:true;title:'Sources'
                    Repeater {model:root.providers;delegate:ColumnLayout {required property var modelData;Layout.fillWidth:true;spacing:4
                        LabelText {text:modelData.name;font.weight:Font.DemiBold}
                        LabelText {Layout.fillWidth:true;text:modelData.available?modelData.count+' items · updated '+root.age(modelData.modified_at):'Not available on this computer';color:Theme.muted;font.pixelSize:12;wrapMode:Text.Wrap}
                    }}
                    SCheckBox {text:'Show the next article every ten seconds';checked:root.cycling;onToggled:root.cycling=checked;Accessible.description:'Paused by default and when you select a source.'}
                }
                SDisclosure {
                    id:managePreferences
                    Layout.fillWidth:true;title:'Manage source preferences'
                    Repeater {model:root.rules.filter(r=>r.kind==='exclude' && r.matcher==='url' && r.enabled);delegate:RowLayout {
                        required property var modelData
                        Layout.fillWidth:true
                        LabelText {text:modelData.pattern;Layout.fillWidth:true;elide:Text.ElideMiddle;Accessible.name:modelData.pattern}
                        SButton {text:'Remove exclusion';quiet:true;onClicked:root.removeRule(modelData)}
                    }}
                    LabelText {visible:!root.rules.some(r=>r.kind==='exclude' && r.matcher==='url' && r.enabled);text:'No source exclusions';color:Theme.muted}
                }
                RowLayout {Layout.fillWidth:true;Item{Layout.fillWidth:true} SButton {text:'Done';primary:true;onClicked:discoveryOptions.close()} }
            }
        }
    }
}
