import { describe, it, expect, beforeAll, afterAll } from "vitest";
import type { VScreenServer } from "./harness";

/**
 * E2E test scenarios for vscreen.
 *
 * These tests require a running vscreen server instance.
 * They exercise the HTTP API and WebSocket signaling.
 */
describe("vscreen E2E", () => {
  // Server instance would be started in beforeAll
  // For now, these are placeholder scenario definitions

  describe("Instance Management", () => {
    it.todo("should create an instance via POST /instances");
    it.todo("should list instances via GET /instances");
    it.todo("should get instance health via GET /instances/:id/health");
    it.todo("should delete an instance via DELETE /instances/:id");
    it.todo("should reject duplicate instance creation");
    it.todo("should enforce max instance limit");
  });

  describe("WebRTC Signaling", () => {
    it.todo("should establish WebSocket connection for signaling");
    it.todo("should receive peer ID on connect");
    it.todo("should exchange SDP offer/answer");
    it.todo("should exchange ICE candidates");
    it.todo("should handle graceful disconnect");
  });

  describe("Runtime Configuration", () => {
    it.todo("should update video config via PATCH /instances/:id/video");
  });

  describe("Error Handling", () => {
    it.todo("should return 404 for nonexistent instance");
    it.todo("should return 409 for duplicate instance");
    it.todo("should return proper JSON error format");
  });
});
