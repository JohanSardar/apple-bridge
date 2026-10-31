#import "ClockNative.h"
#import <math.h>

@interface NSObject (ABClockTimerAPI)
- (id)timers;
- (id)addTimer:(id)timer;
- (id)removeTimer:(id)timer;
- (id)initWithState:(NSUInteger)state duration:(double)duration;
- (id)addSuccessBlock:(void (^)(id))block;
- (id)addFailureBlock:(void (^)(NSError *))block;
- (void)cancel;
- (NSString *)timerIDString;
- (NSString *)title;
- (void)setTitle:(NSString *)title;
- (double)duration;
- (NSUInteger)state;
- (NSDate *)fireDate;
@end

static id awaitResult(id future) {
    if (![future respondsToSelector:@selector(addSuccessBlock:)] || ![future respondsToSelector:@selector(addFailureBlock:)]) return nil;
    NSObject *lock = [NSObject new];
    __block BOOL finished = NO;
    __block id result;
    [future addSuccessBlock:^(id value) { @synchronized(lock) { result = value ?: @YES; finished = YES; } }];
    [future addFailureBlock:^(NSError *error) { @synchronized(lock) { finished = YES; } }];
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:10];
    while (deadline.timeIntervalSinceNow > 0) {
        @synchronized(lock) { if (finished) return result; }
        [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
    }
    if ([future respondsToSelector:@selector(cancel)]) [future cancel];
    return nil;
}

static NSArray *readTimers(id manager) {
    id timers = awaitResult([manager timers]);
    return [timers isKindOfClass:[NSArray class]] ? timers : nil;
}

NSData *ABClockTimersExchange(NSData *requestData) {
    @try {
        NSDictionary *request = [NSJSONSerialization JSONObjectWithData:requestData options:0 error:NULL];
        if (![request isKindOfClass:[NSDictionary class]]) @throw [NSException exceptionWithName:@"invalid" reason:nil userInfo:nil];
        [[NSBundle bundleWithPath:@"/System/Library/PrivateFrameworks/MobileTimer.framework"] load];
        Class managerClass = NSClassFromString(@"MTTimerManager"), timerClass = NSClassFromString(@"MTTimer");
        for (NSString *name in @[@"timers", @"addTimer:", @"removeTimer:"]) {
            if (![managerClass instancesRespondToSelector:NSSelectorFromString(name)]) @throw [NSException exceptionWithName:@"unavailable" reason:nil userInfo:nil];
        }
        for (NSString *name in @[@"initWithState:duration:", @"setTitle:", @"title", @"timerIDString", @"duration", @"state", @"fireDate"]) {
            if (![timerClass instancesRespondToSelector:NSSelectorFromString(name)]) @throw [NSException exceptionWithName:@"unavailable" reason:nil userInfo:nil];
        }
        id manager = [managerClass new];
        NSArray *before = readTimers(manager);
        if (!before) @throw [NSException exceptionWithName:@"unavailable" reason:nil userInfo:nil];
        NSString *operation = request[@"operation"];
        if ([operation isEqual:@"start_timer"]) {
            NSString *label = request[@"label"];
            double duration = [request[@"duration"] doubleValue];
            if (![label isKindOfClass:[NSString class]] || ![label hasPrefix:@"[ab-test]"] || label.length > 256 || duration < 1 || duration > 86400) @throw [NSException exceptionWithName:@"invalid" reason:nil userInfo:nil];
            for (id timer in before) if ([[timer title] isEqual:label]) @throw [NSException exceptionWithName:@"duplicate" reason:nil userInfo:nil];
            id timer = [(id)[timerClass alloc] initWithState:3 duration:duration];
            [timer setTitle:label];
            if (!awaitResult([manager addTimer:timer])) @throw [NSException exceptionWithName:@"unavailable" reason:nil userInfo:nil];
        } else if ([operation isEqual:@"delete_timer"]) {
            NSDictionary *requested = request[@"timer"];
            NSString *identifier = requested[@"id"], *label = requested[@"label"];
            if (![identifier isKindOfClass:[NSString class]] || ![label isKindOfClass:[NSString class]] || ![label hasPrefix:@"[ab-test]"]) @throw [NSException exceptionWithName:@"invalid" reason:nil userInfo:nil];
            for (id timer in before) if ([[timer timerIDString] isEqual:identifier]) {
                if (![[timer title] isEqual:label] || !awaitResult([manager removeTimer:timer])) @throw [NSException exceptionWithName:@"unavailable" reason:nil userInfo:nil];
            }
        } else if (![operation isEqual:@"timers"] && ![operation isEqual:@"capabilities"]) {
            @throw [NSException exceptionWithName:@"invalid" reason:nil userInfo:nil];
        }
        NSArray *after = readTimers([managerClass new]);
        if (!after) @throw [NSException exceptionWithName:@"unavailable" reason:nil userInfo:nil];
        NSMutableArray *rows = [NSMutableArray new];
        for (id timer in after) {
            NSString *identifier = [timer timerIDString], *label = [timer title] ?: @"";
            double duration = [timer duration];
            if (!identifier.length || !isfinite(duration) || duration < 1 || duration > 86400) @throw [NSException exceptionWithName:@"invalid" reason:nil userInfo:nil];
            [rows addObject:@{@"id":identifier, @"label":label, @"duration":@((NSInteger)duration), @"running":@([timer state] == 3 && [timer fireDate].timeIntervalSinceNow > 0)}];
        }
        NSMutableDictionary *output = [@{@"version":@1, @"operation":operation, @"timers":rows, @"capabilities":@[@"timers", @"start_timer", @"delete_timer"]} mutableCopy];
        if ([operation isEqual:@"start_timer"]) {
            for (NSDictionary *row in rows) if ([row[@"label"] isEqual:request[@"label"]]) {
                output[@"timer_running"] = row[@"running"]; output[@"duration"] = row[@"duration"]; output[@"label"] = row[@"label"];
            }
        }
        return [NSJSONSerialization dataWithJSONObject:output options:NSJSONWritingSortedKeys error:NULL];
    } @catch (NSException *exception) {
        return [NSJSONSerialization dataWithJSONObject:@{@"error":@"Clock could not read or update its timers. Open Clock and retry; only tagged test timers can be changed."} options:0 error:NULL];
    }
}
