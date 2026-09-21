import React from "react";
import type { ChatImageAttachment } from "../types";

export interface UserMessageProps {
  content: string;
  images?: ChatImageAttachment[];
}

export function UserMessage({ content, images }: UserMessageProps) {
  return (
    <div
      data-testid="user-message"
      className="w-full relative p-3 my-1.5 rounded-xl bg-zinc-100/90 text-zinc-900 text-[13px] font-sans leading-relaxed whitespace-pre-wrap break-words border border-zinc-200/50 select-text flex flex-col gap-2"
    >
      {images && images.length > 0 && (
        <div
          data-testid="user-message-images"
          className="flex flex-wrap gap-2 pb-1 border-b border-zinc-200/40"
        >
          {images.map((img) => (
            <div
              key={img.id}
              data-testid={`user-message-image-${img.id}`}
              className="relative group rounded-lg overflow-hidden border border-zinc-300/70 bg-white shadow-2xs"
            >
              <img
                src={img.url}
                alt={img.name || "Attached image"}
                className="max-w-[240px] max-h-[180px] object-cover rounded-md"
              />
              {img.name && (
                <div className="absolute bottom-0 inset-x-0 bg-black/60 backdrop-blur-xs text-white text-[10px] px-1.5 py-0.5 truncate">
                  {img.name}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
      <div>{content}</div>
    </div>
  );
}

export default UserMessage;
