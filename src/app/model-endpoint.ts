import type { OverviewApi } from "../lib/tauri-client";

export type ModelEndpoint =
  | { kind: "available"; value: string }
  | { kind: "not-configured" }
  | { kind: "invalid"; reason: string };

export function isHttpUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return (url.protocol === "http:" || url.protocol === "https:") && Boolean(url.hostname);
  } catch {
    return false;
  }
}

export function buildModelEndpoint(
  baseUrl: string | null | undefined,
  modelId: string,
  api: OverviewApi | null | undefined,
): ModelEndpoint {
  const base = baseUrl?.trim();
  if (!base || !api) return { kind: "not-configured" };
  try {
    const endpoint = new URL(base);
    if (endpoint.protocol !== "http:" && endpoint.protocol !== "https:") {
      return { kind: "invalid", reason: "Provider Base URL 必须使用 HTTP(S)" };
    }
    switch (api) {
      case "openai-completions":
        return { kind: "available", value: appendEndpointPath(endpoint, "chat/completions").toString() };
      case "openai-responses":
        return { kind: "available", value: appendEndpointPath(endpoint, "responses").toString() };
      case "anthropic-messages":
        return { kind: "available", value: appendEndpointPath(endpoint, "v1/messages").toString() };
      case "google-generative-ai": {
        const googleEndpoint = appendEndpointPath(endpoint, `models/${encodeURIComponent(modelId)}:streamGenerateContent`);
        googleEndpoint.searchParams.set("alt", "sse");
        return { kind: "available", value: googleEndpoint.toString() };
      }
    }
  } catch {
    return { kind: "invalid", reason: "Provider Base URL 无效或已脱敏" };
  }
  return { kind: "invalid", reason: "有效协议不受支持" };

}
function appendEndpointPath(endpoint: URL, suffix: string): URL {
  const basePath = endpoint.pathname.replace(/\/+$/, "");
  endpoint.pathname = `${basePath}/${suffix}`;
  return endpoint;
}
