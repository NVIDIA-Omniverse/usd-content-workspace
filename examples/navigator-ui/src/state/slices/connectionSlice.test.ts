// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

import { act, renderHook } from "@testing-library/react";
import { Authentication } from "@/services/auth";
import { createMockFile } from "@/test-utils/mockPath";
import { useActiveConnection, useAppStore } from "../appStore";
import { ConnectionInfo } from "./connectionSlice";

describe("connection store", () => {
  const endpoint = "http://test";
  const info: ConnectionInfo = {
    auth: {
      accessToken: "",
      idToken: "",
      refreshToken: "",
      expiresAt: new Date(),
    },
    discovery: "",
  };

  beforeEach(() => {
    act(() => {
      useAppStore.setState(useAppStore.getInitialState());
    });
  });

  it("adds a connection", () => {
    const { result } = renderHook(() => useAppStore());
    act(() => {
      result.current.addConnection(endpoint, info);
    });
    expect(result.current.connections[endpoint]).toBeDefined();
  });

  it("does not add duplicate endpoints", () => {
    const { result } = renderHook(() => useAppStore());
    act(() => {
      result.current.addConnection(endpoint, info);
    });
    act(() => {
      result.current.addConnection(endpoint, info);
    });
    expect(Array.from(result.current.endpoints)).toEqual([endpoint]);
  });

  it("removes a connection", () => {
    const { result } = renderHook(() => useAppStore());
    act(() => {
      result.current.addConnection(endpoint, info);
    });
    act(() => {
      result.current.removeConnection(endpoint);
    });
    expect(result.current.connections[endpoint]).toBeUndefined();
  });

  it("sets a connection active", () => {
    const { result } = renderHook(() => useAppStore());
    act(() => {
      result.current.addConnection(endpoint, info);
    });
    act(() => {
      result.current.setActiveConnection(endpoint);
    });
    expect(result.current.activeConnection).toBe(endpoint);
  });

  it("sets the selected folder for a connection", () => {
    const { result } = renderHook(() => useAppStore());
    act(() => {
      result.current.addConnection(endpoint, info);
    });
    const path = "somePath/";
    act(() => {
      result.current.setCurrentFolder(endpoint, path);
    });
    expect(result.current.connections[endpoint].currentFolder).toBe(path);
  });

  it("sets auth info for a connection", () => {
    const auth = {
      accessToken: "",
      refreshToken: "refresh",
      expiresAt: new Date(2025, 2),
      idToken: "token",
    };

    const { result } = renderHook(() => useAppStore());
    act(() => {
      result.current.addConnection(endpoint, info);
    });
    act(() => {
      result.current.setConnectionAuth(endpoint, auth);
    });
    expect(result.current.connections[endpoint].info.auth).toBe(auth);
  });

  it("returns the active connection", () => {
    const { result: store } = renderHook(() => useAppStore());
    act(() => {
      store.current.addConnection(endpoint, info);
      store.current.setActiveConnection(endpoint);
    });
    const { result: active } = renderHook(() => useActiveConnection());
    expect(active.current).toBeDefined();
    expect(active.current?.endpoint).toBe(endpoint);
  });

  describe("syncDefaultConnections", () => {
    const discovery = "https://stack.example/";
    const auth: Authentication = {
      accessToken: "access",
      idToken: "id",
      refreshToken: "refresh",
      expiresAt: new Date(Date.now() + 3600_000),
    };

    const endpoints: string[] = ["http://storage-1/", "http://storage-2/"];

    it("adds all storage endpoints from the discovery", () => {
      const { result } = renderHook(() => useAppStore());
      act(() => {
        result.current.syncDefaultConnections(discovery, endpoints, auth);
      });

      expect(Array.from(result.current.endpoints).sort()).toEqual(["http://storage-1/", "http://storage-2/"]);
      expect(result.current.connections["http://storage-1/"].info).toEqual({
        auth,
        discovery,
      });
      expect(result.current.connections["http://storage-2/"].info).toEqual({
        auth,
        discovery,
      });
    });

    it("picks the first entry as the active connection when none is set", () => {
      const { result } = renderHook(() => useAppStore());
      act(() => {
        result.current.syncDefaultConnections(discovery, endpoints, auth);
      });
      expect(result.current.activeConnection).toBe("http://storage-1/");
    });

    it("removes connections that are no longer advertised by discovery", () => {
      const { result } = renderHook(() => useAppStore());
      act(() => {
        result.current.addConnection("http://stale/", { ...info, discovery });
        result.current.syncDefaultConnections(discovery, endpoints, auth);
      });

      expect(result.current.connections["http://stale/"]).toBeUndefined();
      expect(result.current.endpoints.has("http://stale/")).toBe(false);
    });

    it("preserves currentFolder and selectedPaths on a matching endpoint", () => {
      const { result } = renderHook(() => useAppStore());
      const existingAuth: Authentication = {
        accessToken: "older-but-still-valid",
        idToken: "id",
        refreshToken: "refresh",
        expiresAt: new Date(Date.now() + 3600_000),
      };
      const selectedFile = createMockFile("http://storage-1/some/folder/file");
      act(() => {
        result.current.addConnection("http://storage-1/", { auth: existingAuth, discovery });
        result.current.setCurrentFolder("http://storage-1/", "some/folder/");
        result.current.setSelectedPaths("http://storage-1/", [selectedFile]);
      });

      act(() => {
        result.current.syncDefaultConnections(discovery, endpoints, auth);
      });

      const preserved = result.current.connections["http://storage-1/"];
      expect(preserved.currentFolder).toBe("some/folder/");
      expect(Array.from(preserved.selectedPaths)).toEqual([selectedFile]);
      expect(preserved.info.auth).toBe(auth);
    });

    it("clears the active connection when discovery returns no rest endpoints", () => {
      const { result } = renderHook(() => useAppStore());
      act(() => {
        result.current.syncDefaultConnections(discovery, endpoints, auth);
      });
      act(() => {
        result.current.syncDefaultConnections(discovery, [], auth);
      });

      expect(result.current.activeConnection).toBe("");
      expect(result.current.connections).toEqual({});
    });
  });
});
