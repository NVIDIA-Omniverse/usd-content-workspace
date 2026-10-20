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

import { createTheme, Modal, rem } from "@mantine/core";

export const theme = createTheme({
  breakpoints: {
    xs: "30em",
    sm: "48em",
    md: "64em",
    lg: "74em",
    xl: "90em",
  },
  fontFamily: "Open Sans, sans-serif",
  defaultRadius: "sm",
  fontWeights: {
    medium: "500",
  },
  fontSizes: {
    xs: rem(12),
    sm: rem(13),
    md: rem(14),
    lg: rem(15),
    xl: rem(16),
  },
  colors: {
    green: [
      "#76b900",
      "#76b900",
      "#76b900",
      "#76b900",
      "#76b900",
      "#76b900",
      "#76b900",
      "#76b900",
      "#76b900",
      "#76b900",
    ],
  },

  white: "#fff",

  headings: {
    sizes: {
      h1: {
        fontSize: rem(18),
        fontWeight: "bold",
      },
      h2: {
        fontSize: rem(16),
        fontWeight: "bold",
      },
      h3: {
        fontSize: rem(14),
      },
      h4: {
        fontSize: rem(12),
      },
      h5: {
        fontSize: rem(12),
      },
      h6: {
        fontSize: rem(12),
      },
    },
  },

  components: {
    Modal: Modal.extend({
      styles: {
        title: {
          fontWeight: "bold",
        },
      },
    }),
  },
});
