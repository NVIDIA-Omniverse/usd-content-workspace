# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0


class AnyInstanceOf:
    """A helper object that compares equal to every instance of the specified class."""

    def __init__(self, cls):
        self.cls = cls

    def __eq__(self, other):
        return isinstance(other, self.cls)

    def __ne__(self, other):
        return not isinstance(other, self.cls)

    def __repr__(self):
        return f"<ANY {self.cls.__name__}>"
