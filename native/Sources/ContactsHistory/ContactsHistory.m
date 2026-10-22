#import "ContactsHistory.h"

@interface AppleBridgeContactsHistoryBatch ()
@property(nonatomic, readwrite, copy) NSData *token;
@property(nonatomic, readwrite) BOOL droppedEverything;
@property(nonatomic, readwrite) BOOL requiresFullScan;
@property(nonatomic, readwrite, copy) NSArray<NSString *> *fullScanEventKinds;
@property(nonatomic, readwrite, copy) NSArray<NSString *> *changedContactIdentifiers;
@property(nonatomic, readwrite, copy) NSArray<NSString *> *changedGroupIdentifiers;
@end

@implementation AppleBridgeContactsHistoryBatch
@end

@implementation AppleBridgeContactsHistory
+ (nullable AppleBridgeContactsHistoryBatch *)fetchStore:(CNContactStore *)store request:(CNChangeHistoryFetchRequest *)request {
    @try {
        NSError *error = nil;
        CNFetchResult<NSEnumerator<CNChangeHistoryEvent *> *> *result = [store enumeratorForChangeHistoryFetchRequest:request error:&error];
        if (result == nil || error != nil) { return nil; }
        BOOL dropped = NO;
        BOOL fullScan = NO;
        NSMutableSet<NSString *> *changed = [NSMutableSet new];
        NSMutableSet<NSString *> *fullScanKinds = [NSMutableSet new];
        NSMutableSet<NSString *> *groups = [NSMutableSet new];
        for (CNChangeHistoryEvent *event in result.value) {
            if ([event isKindOfClass:[CNChangeHistoryDropEverythingEvent class]]) { dropped = YES; }
            else if ([event isKindOfClass:[CNChangeHistoryAddContactEvent class]]) { [changed addObject:((CNChangeHistoryAddContactEvent *)event).contact.identifier]; }
            else if ([event isKindOfClass:[CNChangeHistoryUpdateContactEvent class]]) { [changed addObject:((CNChangeHistoryUpdateContactEvent *)event).contact.identifier]; }
            else if ([event isKindOfClass:[CNChangeHistoryDeleteContactEvent class]]) { [changed addObject:((CNChangeHistoryDeleteContactEvent *)event).contactIdentifier]; }
            else if ([event isKindOfClass:[CNChangeHistoryAddGroupEvent class]]) { [groups addObject:((CNChangeHistoryAddGroupEvent *)event).group.identifier]; }
            else if ([event isKindOfClass:[CNChangeHistoryUpdateGroupEvent class]]) { [groups addObject:((CNChangeHistoryUpdateGroupEvent *)event).group.identifier]; }
            else if ([event isKindOfClass:[CNChangeHistoryDeleteGroupEvent class]]) { [groups addObject:((CNChangeHistoryDeleteGroupEvent *)event).groupIdentifier]; }
            else if ([event isKindOfClass:[CNChangeHistoryAddMemberToGroupEvent class]]) { [groups addObject:((CNChangeHistoryAddMemberToGroupEvent *)event).group.identifier]; }
            else if ([event isKindOfClass:[CNChangeHistoryRemoveMemberFromGroupEvent class]]) { [groups addObject:((CNChangeHistoryRemoveMemberFromGroupEvent *)event).group.identifier]; }
            else if ([event isKindOfClass:[CNChangeHistoryAddSubgroupToGroupEvent class]]) { [groups addObject:((CNChangeHistoryAddSubgroupToGroupEvent *)event).group.identifier]; }
            else if ([event isKindOfClass:[CNChangeHistoryRemoveSubgroupFromGroupEvent class]]) { [groups addObject:((CNChangeHistoryRemoveSubgroupFromGroupEvent *)event).group.identifier]; }
            else { fullScan = YES; [fullScanKinds addObject:NSStringFromClass(event.class)]; }
        }
        if (result.currentHistoryToken.length == 0) { return nil; }
        AppleBridgeContactsHistoryBatch *batch = [AppleBridgeContactsHistoryBatch new];
        batch.token = result.currentHistoryToken;
        batch.droppedEverything = dropped;
        batch.requiresFullScan = fullScan;
        batch.fullScanEventKinds = [fullScanKinds.allObjects sortedArrayUsingSelector:@selector(compare:)];
        batch.changedContactIdentifiers = changed.allObjects;
        batch.changedGroupIdentifiers = groups.allObjects;
        return batch;
    } @catch (NSException *exception) {
        (void)exception;
        return nil;
    }
}
@end
