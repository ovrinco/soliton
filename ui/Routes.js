.pragma library

function normalize(page, view) {
    var defaults = {Dashboard:'Timeline', Goals:'Goals', Tasks:'Tasks', Notes:'Notes',
                    Learning:'Feed', Strategy:'Overview', Portal:'Devices', Settings:'General',
                    Hearth:'Habits'};
    if (page === 'Dashboards') page = 'Dashboard';
    if (page === 'Learning Center') page = 'Learning';
    if (page === 'Developer Center' || page === 'Modules') { page = 'Strategy'; view = 'Modules'; }
    if (page === 'Action Center' || page === 'Dashboard') {
        if (view === 'Tasks') page = 'Tasks';
        else if (view === 'Categories & Goals' || view === 'Goals') { page = 'Goals'; view = 'Goals'; }
        else if (view === 'Notes') page = 'Notes';
        else if (view === 'Strategy') { page = 'Strategy'; view = 'Ideas'; }
        else { page = 'Dashboard'; if (view === 'Calendar') view = 'Day'; }
    }
    if (!defaults[page]) page = 'Dashboard';
    if (page === 'Settings' && view === 'Control') view = 'General';
    // Hearth holds its modules that have landed; it opens on Habits.
    if (page === 'Hearth' && ['Habits', 'Kitchen', 'Finance', 'Now'].indexOf(view) < 0) view = 'Habits';
    var views = pageViews(page);
    if (views.length && page !== 'Strategy' && page !== 'Settings' && views.indexOf(view) < 0) view = defaults[page];
    return {page:page, view:view || defaults[page]};
}

// The views a page lists in the side panel, in the order of its former tab strip.
function pageViews(page) {
    return {Learning:['Feed', 'YouTube Intel', 'Notes', 'Sources'],
            Strategy:['Overview', 'Ideas', 'Execution', 'Tracker', 'Chats', 'Voice notes', 'Runs', 'Modules'],
            Portal:['Devices', 'Transfers', 'Media'],
            Settings:['General', 'Monitors', 'Key bindings', 'Plugins', 'Skills', 'Zeno', 'Log', 'Processes']}[page] || [];
}

function supportsDetails(page, view) {
    return ['Dashboard','Goals','Tasks','Notes'].indexOf(page) >= 0 ||
           page === 'Strategy' && view === 'Ideas' || page === 'Learning' && view === 'Notes';
}
