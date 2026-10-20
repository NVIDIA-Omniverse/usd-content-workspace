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

import { useEffect, useRef } from "react";
import { Navigate, useLocation } from "react-router-dom";
import { useShallow } from "zustand/react/shallow";
import { AppShell, Splitter } from "@mantine/core";
import type { SplitterPaneSize } from "@mantine/hooks";
import { Notifications } from "@mantine/notifications";
import { Header } from "@/components/Header";
import { PageLoader } from "@/components/PageLoader";
import { PathContent } from "@/components/PathContent";
import { PathCurrentFolderDetails } from "@/components/PathCurrentFolderDetails";
import { PathDetails } from "@/components/PathDetails";
import { PathSelectionDetails } from "@/components/PathSelectionDetails";
import { PathTree } from "@/components/PathTree";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useDefaultServerBootstrap } from "@/hooks/useDefaultServerBootstrap";
import { useNotification } from "@/hooks/useNotification";
import { constructPathNavLink, usePathNavigation } from "@/hooks/usePathNavigation";
import { isHttpError } from "@/services/api";
import { useActiveConnection, useAppStore } from "@/state/appStore";
import styles from "./Main.module.css";

export function Main() {
  usePathNavigation();

  const { defaultDiscovery } = useAppConfig();
  const { pathname } = useLocation();
  const activeConnection = useAppStore((state) => state.activeConnection);
  const bootstrap = useDefaultServerBootstrap();
  const notifications = useNotification();
  const reportedErrorRef = useRef<unknown>(null);
  const [treePanelWidth, setTreePanelWidth] = useAppStore(
    useShallow((state) => [state.treePanelWidth, state.setTreePanelWidth])
  );

  const detailsVisible = useAppStore((state) => state.detailsPanel);
  const [detailsPanelWidth, setDetailsPanelWidth] = useAppStore(
    useShallow((state) => [state.detailsPanelWidth, state.setDetailsPanelWidth])
  );

  const handleResize = (_handleIndex: number, sizes: SplitterPaneSize[]) => {
    setTreePanelWidth(sizes[0]);
    // sizes will not contain a size for the details panel if it's not visible
    if (detailsVisible) {
      setDetailsPanelWidth(sizes[sizes.length - 1]);
    }
  };

  const connection = useActiveConnection();
  const selectedPaths = connection?.selectedPaths ?? new Set();

  // 401/403 responses from discovery are the trigger for the OIDC redirect performed
  // inside `api.ts`; the page is about to navigate away to the IdP, so they aren't
  // surfaced as user-visible errors. Genuine failures (network, 5xx, parse) still are.
  const isAuthRedirectError =
    bootstrap.isError && isHttpError(bootstrap.error) && [401, 403].includes(bootstrap.error.response.status);

  useEffect(() => {
    if (!bootstrap.isError || !bootstrap.error || reportedErrorRef.current === bootstrap.error) {
      return;
    }
    if (isAuthRedirectError) {
      return;
    }
    reportedErrorRef.current = bootstrap.error;
    notifications.show({
      type: "error",
      title: `Failed to connect to ${defaultDiscovery}`,
      message: bootstrap.error instanceof Error ? bootstrap.error.message : String(bootstrap.error),
    });
  }, [bootstrap.error, bootstrap.isError, defaultDiscovery, isAuthRedirectError, notifications]);

  // In default-server mode the layout is not "ready" until the bootstrap query finishes
  // (success path, or non-auth error). While pending or while the api layer is in the
  // middle of redirecting to the IdP, keep the centered spinner visible so the user
  // never sees the empty "No active connection." state nor a partially populated tree.
  const isBootstrapping = !!defaultDiscovery && (bootstrap.isPending || isAuthRedirectError);

  // Once the bootstrap has produced an active connection, route the bare `/` URL to
  // that connection so the rest of the app works against a real `:endpoint/path/...` route.
  const shouldRedirectToActive = !!defaultDiscovery && pathname === "/" && !!activeConnection;

  if (isBootstrapping) {
    return <PageLoader label="Connecting to default server" />;
  }

  return (
    <>
      <Notifications limit={5} />
      {shouldRedirectToActive && <Navigate to={constructPathNavLink(activeConnection)} replace />}
      <AppShell header={headerProps} padding={0}>
        <AppShell.Header style={{ overflow: "hidden" }}>
          <Header />
        </AppShell.Header>
        <AppShell.Main h="calc(100vh - var(--app-shell-header-height, 0px))">
          <Splitter
            // Remount when the pane count changes: the Splitter seeds its internal sizes from the
            // pane list only on mount, so a pane added later would otherwise render at width 0.
            key={detailsVisible ? "with-details" : "without-details"}
            h="100%"
            withHandle={false}
            handleColor="var(--mantine-color-default-border)"
            lineSize={1}
            classNames={{ handle: styles.splitterHandle }}
            onResizeEnd={handleResize}
          >
            <Splitter.Pane defaultSize={treePanelWidth} min="300px">
              <PathTree />
            </Splitter.Pane>

            <Splitter.Pane defaultSize={50} min="200px" miw="200px">
              <PathContent />
            </Splitter.Pane>

            {detailsVisible && (
              <Splitter.Pane defaultSize={detailsPanelWidth} min="450px" max="650px">
                {selectedPaths.size > 1 ? (
                  <PathSelectionDetails paths={selectedPaths} />
                ) : selectedPaths.size === 1 && connection ? (
                  <PathDetails path={Array.from(selectedPaths)[0]} connection={connection} />
                ) : (
                  connection && <PathCurrentFolderDetails connection={connection} />
                )}
              </Splitter.Pane>
            )}
          </Splitter>
        </AppShell.Main>
      </AppShell>
    </>
  );
}

const headerProps = {
  height: 60,
};
