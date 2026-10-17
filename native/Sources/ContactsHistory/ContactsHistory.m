#import "ContactsHistory.h"

@interface AppleBridgeContactsHistoryBatch ()
@property(nonatomic, readwrite, copy) NSData *token;
@property(nonatomic, readwrite) BOOL droppedEverything;
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
        for (CNChangeHistoryEvent *event in result.value) {
            if ([event isKindOfClass:[CNChangeHistoryDropEverythingEvent class]]) { dropped = YES; }
        }
        if (result.currentHistoryToken.length == 0) { return nil; }
        AppleBridgeContactsHistoryBatch *batch = [AppleBridgeContactsHistoryBatch new];
        batch.token = result.currentHistoryToken;
        batch.droppedEverything = dropped;
        return batch;
    } @catch (NSException *exception) {
        (void)exception;
        return nil;
    }
}
@end
