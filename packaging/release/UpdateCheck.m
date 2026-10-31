#import <AppKit/AppKit.h>
#import <Sparkle/Sparkle.h>

@interface UpdateCheck : NSObject <SPUUserDriver, SPUUpdaterDelegate>
@property(strong) SPUUpdater *updater;
@end

@implementation UpdateCheck
- (void)showUpdatePermissionRequest:(SPUUpdatePermissionRequest *)request reply:(void (^)(SUUpdatePermissionResponse *))reply { reply([[SUUpdatePermissionResponse alloc] initWithAutomaticUpdateChecks:NO sendSystemProfile:NO]); }
- (void)showUserInitiatedUpdateCheckWithCancellation:(void (^)(void))cancellation {}
- (void)showUpdateFoundWithAppcastItem:(SUAppcastItem *)item state:(SPUUserUpdateState *)state reply:(void (^)(SPUUserUpdateChoice))reply { reply(SPUUserUpdateChoiceInstall); }
- (void)showUpdateReleaseNotesWithDownloadData:(SPUDownloadData *)data {}
- (void)showUpdateReleaseNotesFailedToDownloadWithError:(NSError *)error {}
- (void)showUpdateNotFoundWithError:(NSError *)error acknowledgement:(void (^)(void))reply { NSLog(@"Update check failed: %@", error); exit(1); }
- (void)showUpdaterError:(NSError *)error acknowledgement:(void (^)(void))reply { NSLog(@"Update installation failed: %@", error); exit(1); }
- (void)showDownloadInitiatedWithCancellation:(void (^)(void))cancellation {}
- (void)showDownloadDidReceiveExpectedContentLength:(uint64_t)length {}
- (void)showDownloadDidReceiveDataOfLength:(uint64_t)length {}
- (void)showDownloadDidStartExtractingUpdate {}
- (void)showExtractionReceivedProgress:(double)progress {}
- (void)showReadyToInstallAndRelaunch:(void (^)(SPUUserUpdateChoice))reply { reply(SPUUserUpdateChoiceInstall); }
- (void)showInstallingUpdateWithApplicationTerminated:(BOOL)terminated retryTerminatingApplication:(void (^)(void))retry { if (!terminated) [NSApp terminate:nil]; }
- (void)showUpdateInstalledAndRelaunched:(BOOL)relaunched acknowledgement:(void (^)(void))reply { reply(); puts("R1-SPARKLE-INSTALL passed=true"); exit(0); }
- (void)dismissUpdateInstallation { exit(1); }
- (BOOL)updaterShouldPromptForPermissionToCheckForUpdates:(SPUUpdater *)updater { return NO; }
@end

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc != 2) return 2;
        [NSApplication sharedApplication];
        NSBundle *host = [NSBundle bundleWithPath:[NSString stringWithUTF8String:argv[1]]];
        if (![host.bundleIdentifier hasPrefix:@"dev.applebridge.r1test."] || ![host.bundlePath hasPrefix:[NSHomeDirectory() stringByAppendingPathComponent:@"Library/Caches/ab-test-update-"]]) return 2;
        UpdateCheck *driver = [UpdateCheck new];
        driver.updater = [[SPUUpdater alloc] initWithHostBundle:host applicationBundle:NSBundle.mainBundle userDriver:driver delegate:driver];
        NSError *error = nil;
        if (![driver.updater startUpdater:&error]) { NSLog(@"Update startup failed: %@", error); return 1; }
        [driver.updater checkForUpdates];
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 120 * NSEC_PER_SEC), dispatch_get_main_queue(), ^{ exit(1); });
        [NSApp run];
    }
    return 1;
}
