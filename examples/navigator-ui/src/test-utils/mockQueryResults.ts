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

import { InfiniteData, UseInfiniteQueryResult } from "@tanstack/react-query";
import { vi } from "vitest";

export function createMockInfiniteQueryResult<TData = unknown, TError = Error, TPageParam = unknown>(
  pageData?: TData,
  options: Partial<UseInfiniteQueryResult<InfiniteData<TData, TPageParam>, TError>> = {}
): UseInfiniteQueryResult<InfiniteData<TData, TPageParam>, TError> {
  const defaultData: InfiniteData<TData, TPageParam> = {
    pages: pageData ? [pageData] : [],
    pageParams: [null as TPageParam],
  };

  return {
    data: options.data ?? defaultData,
    dataUpdatedAt: Date.now(),
    error: options.error ?? null,
    errorUpdatedAt: Date.now(),
    failureCount: 0,
    failureReason: null,
    errorUpdateCount: 0,
    isError: options.isError ?? false,
    isFetched: options.isFetched ?? true,
    isFetchedAfterMount: options.isFetchedAfterMount ?? true,
    isFetching: options.isFetching ?? false,
    isPaused: options.isPaused ?? false,
    isLoading: options.isLoading ?? false,
    isLoadingError: options.isLoadingError ?? false,
    isPlaceholderData: options.isPlaceholderData ?? false,
    isPending: options.isPending ?? false,
    isRefetchError: options.isRefetchError ?? false,
    isRefetching: options.isRefetching ?? false,
    isStale: options.isStale ?? false,
    isSuccess: options.isSuccess ?? true,
    refetch: options.refetch ?? vi.fn(),
    status: options.status ?? "success",
    fetchNextPage: options.fetchNextPage ?? vi.fn(),
    fetchPreviousPage: options.fetchPreviousPage ?? vi.fn(),
    hasNextPage: options.hasNextPage ?? false,
    hasPreviousPage: options.hasPreviousPage ?? false,
    isFetchingNextPage: options.isFetchingNextPage ?? false,
    isFetchingPreviousPage: options.isFetchingPreviousPage ?? false,
    isFetchNextPageError: options.isFetchNextPageError ?? false,
    isFetchPreviousPageError: options.isFetchPreviousPageError ?? false,
    isInitialLoading: options.isInitialLoading ?? false,
    fetchStatus: options.fetchStatus ?? "idle",
    remove: vi.fn(),
    promise: Promise.resolve(defaultData),
  } as UseInfiniteQueryResult<InfiniteData<TData, TPageParam>, TError>;
}
