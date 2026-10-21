#import <AVFoundation/AVFoundation.h>

@interface ABMediaExportGuard : NSObject
+ (NSError * _Nullable)start:(AVAssetExportSession * _Nonnull)session outputURL:(NSURL * _Nonnull)url fileType:(AVFileType _Nonnull)type completion:(void (^ _Nonnull)(NSError * _Nullable))completion;
@end
