#import <Foundation/Foundation.h>
#import <Contacts/Contacts.h>

NS_ASSUME_NONNULL_BEGIN

@interface AppleBridgeContactsHistoryBatch : NSObject
@property(nonatomic, readonly, copy) NSData *token;
@property(nonatomic, readonly) BOOL droppedEverything;
@end

@interface AppleBridgeContactsHistory : NSObject
+ (nullable AppleBridgeContactsHistoryBatch *)fetchStore:(CNContactStore *)store request:(CNChangeHistoryFetchRequest *)request NS_SWIFT_NAME(fetch(store:request:));
@end

NS_ASSUME_NONNULL_END
