//! Usecase: the prices for a visitor's choice. Nothing is stored or read:
//! the calculator has no IO at all.

const std = @import("std");
const domain = @import("domain.zig");
const Billing = domain.Billing;
const Plan = domain.Plan;
const PlanQuote = domain.PlanQuote;
const Quote = domain.Quote;
const TeamSize = domain.TeamSize;

/// The plans side by side for `users`, or only Enterprise above 250 users.
/// Without a valid team size (null), the plans with their prices per user.
pub fn quoteU(users: ?TeamSize, billing: Billing, ai: bool) Quote {
    if (users) |u| if (u.isEnterprise()) return .{ .enterprise = u };
    var plans: [Plan.all.len]PlanQuote = undefined;
    for (Plan.all, &plans) |plan, *q| q.* = .init(plan, billing, ai, users);
    return .{ .plans = plans };
}

const testing = std.testing;

fn users_(n: []const u8) ?TeamSize {
    return TeamSize.parse(n) catch unreachable;
}

test "up to 250 shows all three plans" {
    const plans = quoteU(users_("250"), .yearly, false).plans;
    for (plans, Plan.all) |q, plan| try testing.expectEqual(plan, q.plan);
}

test "above 250 shows only enterprise" {
    try testing.expectEqual(Quote{ .enterprise = users_("251").? }, quoteU(users_("251"), .yearly, true));
}

test "no team size shows the plans without totals" {
    for (quoteU(null, .yearly, false).plans) |q| try testing.expectEqual(.unknown, std.meta.activeTag(q.team));
}
