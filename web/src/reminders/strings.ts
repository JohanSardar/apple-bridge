export const smartNames: Record<string, string> = { today: 'Today', scheduled: 'Scheduled', all: 'All', flagged: 'Flagged', completed: 'Completed', assigned: 'Assigned to Me' };
export const priorities: Record<number, string> = { 1: 'High', 5: 'Medium', 9: 'Low' };
export const reminderStrings = {
    app: 'Reminders', searchResults: 'Search Results', searchLabel: 'Search Reminders', search: 'Search',
    closeDetails: 'Close details', detailsLabel: 'Reminder details', details: 'Details', readOnly: 'Read only',
    selectList: 'Select a list', back: 'Back to lists', myLists: 'My Lists', tags: 'Tags', smartLists: 'Smart and pinned lists',
    showCompleted: 'Show Completed', sort: 'Sort', manual: 'Manual', dueDate: 'Due Date', created: 'Creation Date', priority: 'Priority', title: 'Title', none: 'None',
    offline: 'Your Mac is unavailable. Reconnecting…', listError: 'Unable to load your lists.', itemsError: 'Unable to load these reminders.',
    capacity: 'This list is too large to show here yet.', capacityHelp: 'Choose a smaller list or tag.', loading: 'Loading reminders…', empty: 'No Reminders',
    unavailable: 'Unavailable', unsupportedFilter: 'Unsupported filter', unsupportedFilterHelp: 'This smart filter is unsupported',
    incomplete: 'Incomplete', completed: 'Completed', flagged: 'Flagged', repeats: 'Repeats', location: 'Location reminder',
    assignedMe: 'Assigned to Me', assigned: 'Assigned', me: 'Me', other: 'Another person', unknownIdentity: 'Identity unavailable',
    image: 'Reminder image', attachment: 'Attachment', list: 'List', due: 'Due', flag: 'Flag', early: 'Early reminder', assignedTo: 'Assigned to'
};
export const reminderCount = (count: number) => `${count} reminder${count === 1 ? '' : 's'}`;
export const priorityLabel = (priority: number) => `${priorities[priority]} priority`;
export const earlyLabel = (delta: number) => `${Math.abs(delta) / 60} minutes before`;

export const subtaskLabel = (count: number) => `${count} subtask${count === 1 ? '' : 's'}`;
export const disclosureLabel = (title: string, expanded: boolean) => `${expanded ? 'Collapse' : 'Expand'} subtasks for ${title}`;

export const incompleteCount = (count: number) => `${count} incomplete reminder${count === 1 ? '' : 's'}`;
export const completedCount = (count: number) => `${count} completed reminder${count === 1 ? '' : 's'}`;
