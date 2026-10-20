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

import React, { useState } from "react";
import { Dropzone, FileWithPath } from "@mantine/dropzone";
import { useFileUpload } from "@/hooks/useFileUpload";
import { Connection, rootFolderPath } from "@/state/slices/connectionSlice";
import styles from "./UploadDropzone.module.css";

export interface UploadDropzoneProps {
  connection: Connection;
  children: React.ReactNode;
  /** Optional extra class (e.g. to suppress focus ring when used as table container) */
  className?: string;
}

export function UploadDropzone({ connection, children, className = "" }: UploadDropzoneProps) {
  const { uploadFromDropzone } = useFileUpload();
  const [dragHover, setDragHover] = useState(false);
  const isRootFolder = connection.currentFolder === rootFolderPath;

  return (
    <Dropzone
      activateOnClick={false}
      className={`${styles.dropzone} ${className}`.trim()}
      disabled={isRootFolder}
      onDrop={(files: FileWithPath[]) => {
        void uploadFromDropzone(files);
        setDragHover(false);
      }}
      onDragEnter={() => setDragHover(true)}
      onDragLeave={() => setDragHover(false)}
    >
      {children}
      {dragHover && !isRootFolder && (
        <div className={styles.dropzoneOverlay}>
          <div className={styles.dropzoneMessage}>Drop files / folders here to upload</div>
        </div>
      )}
    </Dropzone>
  );
}
