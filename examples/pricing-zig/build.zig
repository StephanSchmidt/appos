//! `appos build` runs `zig build --release` here: the app for
//! wasm32-freestanding, with the `wit` module (bindings/wit.zig, written by
//! appos build from wit/) and zt for the templates. `zig build test` runs
//! the unit tests on this machine.

const std = @import("std");
const zt = @import("zt");

pub fn build(b: *std.Build) void {
    const wasm = b.resolveTargetQuery(.{ .cpu_arch = .wasm32, .os_tag = .freestanding });
    const optimize = b.standardOptimizeOption(.{ .preferred_optimize_mode = .safe });
    const zt_wasm = b.dependency("zt", .{ .target = wasm, .optimize = optimize });
    const templates = zt.addTemplates(b, zt_wasm, &.{b.path("src/calculator/view.zt")});

    const app = b.addExecutable(.{ .name = "pricing-zig", .root_module = module(b, "src/main.zig", wasm, optimize, zt_wasm) });
    // A reactor: the shell calls the exports, there is no main.
    app.entry = .disabled;
    app.rdynamic = true;
    app.root_module.strip = true;
    app.step.dependOn(templates);
    b.installArtifact(app);

    const host = b.graph.host;
    const zt_host = b.dependency("zt", .{ .target = host, .optimize = .debug });
    const tests = b.addTest(.{ .root_module = module(b, "src/tests.zig", host, .debug, zt_host) });
    tests.step.dependOn(templates);
    b.step("test", "Run the unit tests").dependOn(&b.addRunArtifact(tests).step);
}

fn module(b: *std.Build, root: []const u8, target: std.Build.ResolvedTarget, optimize: std.builtin.OptimizeMode, zt_dep: *std.Build.Dependency) *std.Build.Module {
    const wit = b.createModule(.{ .root_source_file = b.path("bindings/wit.zig"), .target = target, .optimize = optimize });
    return b.createModule(.{
        .root_source_file = b.path(root),
        .target = target,
        .optimize = optimize,
        .imports = &.{
            .{ .name = "wit", .module = wit },
            .{ .name = "zt", .module = zt_dep.module("zt") },
        },
    });
}
