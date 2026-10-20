/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
import { createBrowserRouter } from "react-router-dom";
import { Diagnostics } from "./pages/Diagnostics";
import { GroupDetails } from "./pages/GroupDetails";
import { GroupList } from "./pages/GroupList";
import { Home } from "./pages/Home";
import { Main } from "./pages/Main";
import { NotFound } from "./pages/NotFound";
import { OpenId } from "./pages/OpenId";
import { PolicyDetails } from "./pages/PolicyDetails";
import { PolicyEdit } from "./pages/PolicyEdit";
import { PolicyForm } from "./pages/PolicyForm";
import { PolicyList } from "./pages/PolicyList";
import { ServiceDetails } from "./pages/ServiceDetails";
import { ServiceForm } from "./pages/ServiceForm";
import { ServiceList } from "./pages/ServiceList";
import { UserDetails } from "./pages/UserDetails";
import { UserList } from "./pages/UserList";

export const router = createBrowserRouter([
  {
    path: "/",
    element: <Main />,
    children: [
      {
        index: true,
        element: <Home />,
      },
      {
        path: "policies/new",
        element: <PolicyForm />,
      },
      {
        path: "policies/:policyId/edit",
        element: <PolicyEdit />,
      },
      {
        path: "policies/:policyId",
        element: <PolicyDetails />,
      },
      {
        path: "policies",
        element: <PolicyList />,
      },
      {
        path: "services/new",
        element: <ServiceForm />,
      },
      {
        path: "services/:service",
        element: <ServiceDetails />,
      },
      {
        path: "services",
        element: <ServiceList />,
      },
      {
        path: "users/:userId",
        element: <UserDetails />,
      },
      {
        path: "users",
        element: <UserList />,
      },
      {
        path: "groups/:groupId",
        element: <GroupDetails />,
      },
      {
        path: "groups",
        element: <GroupList />,
      },
      {
        path: "diagnostics",
        element: <Diagnostics />,
      },
      {
        path: "openid",
        element: <OpenId />,
      },
      {
        path: "*",
        element: <NotFound />,
      },
    ],
  },
]);
