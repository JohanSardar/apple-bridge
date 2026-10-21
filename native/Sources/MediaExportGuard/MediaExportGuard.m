#import "MediaExportGuard.h"

@implementation ABMediaExportGuard
+ (NSError *)start:(AVAssetExportSession *)session outputURL:(NSURL *)url fileType:(AVFileType)type completion:(void (^)(NSError *))completion {
    @try {
        session.outputFileType = type;
        session.outputURL = url;
        [session exportAsynchronouslyWithCompletionHandler:^{
            if (session.status == AVAssetExportSessionStatusCompleted) { completion(nil); }
            else if (session.status == AVAssetExportSessionStatusCancelled) { completion([NSError errorWithDomain:NSCocoaErrorDomain code:NSUserCancelledError userInfo:nil]); }
            else { completion(session.error ?: [NSError errorWithDomain:@"AppleBridgeMediaExport" code:1 userInfo:@{@"reason": @"Export did not complete"}]); }
        }];
        return nil;
    } @catch (NSException *exception) {
        return [NSError errorWithDomain:@"AppleBridgeMediaExport" code:1 userInfo:@{@"exception_name": exception.name, @"reason": exception.reason ?: @"Export configuration exception"}];
    }
}
@end
