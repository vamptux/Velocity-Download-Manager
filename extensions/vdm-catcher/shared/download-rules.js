(function initDownloadRules(globalScope) {
  const ALWAYS_INTERCEPT_EXTENSIONS = new Set([
    "zip", "rar", "7z", "tar", "gz", "bz2", "xz", "lz4", "zst", "cab",
    "exe", "msi", "dmg", "pkg", "deb", "rpm", "apk", "appimage",
    "iso", "img", "bin", "vmdk", "vhd", "vhdx",
    "mp4", "mkv", "mov", "avi", "webm", "m4v", "ts", "m2ts", "flv", "wmv",
    "mp3", "flac", "wav", "ogg", "m4a", "aac", "opus",
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "svg", "avif", "tif", "tiff", "ico", "heic", "heif",
    "pdf", "epub", "mobi",
    "part", "r00", "r01",
  ]);

  const ALWAYS_INTERCEPT_MIMES = new Set([
    "application/zip",
    "application/x-rar-compressed",
    "application/x-rar",
    "application/x-7z-compressed",
    "application/octet-stream",
    "application/x-msdownload",
    "application/x-iso9660-image",
    "video/mp4",
    "video/x-matroska",
    "video/quicktime",
    "video/webm",
    "video/x-msvideo",
    "audio/mpeg",
    "audio/aac",
    "audio/flac",
    "audio/x-m4a",
    "audio/ogg",
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/svg+xml",
    "image/avif",
    "image/heic",
    "image/heif",
    "image/tiff",
    "image/x-icon",
    "application/x-deb",
    "application/x-apple-diskimage",
  ]);

  const VIEWABLE_MEDIA_EXTENSIONS = new Set([
    "mp4", "mkv", "mov", "avi", "webm", "m4v", "ts", "m2ts", "flv", "wmv",
    "mp3", "flac", "wav", "ogg", "m4a", "aac", "opus",
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "svg", "avif", "tif", "tiff", "ico", "heic", "heif",
  ]);

  const DEFAULT_MIN_SIZE_BYTES = 1 * 1024 * 1024;

  const CDN_PATTERNS = [
    /\.cloudfront\.net$/i,
    /\.akamaihd\.net$/i,
    /\.akamai\.net$/i,
    /\.fastly\.net$/i,
    /\.fastlylb\.net$/i,
    /storage\.googleapis\.com/i,
    /s3[.-][a-z0-9-]+\.amazonaws\.com/i,
    /\.blob\.core\.windows\.net/i,
    /cdn[.-]/i,
    /download[.-]/i,
    /dl[.-]/i,
    /files?\./i,
    /static\./i,
    /assets\./i,
    /media\./i,
    /releases?\.github(usercontent)?\.com/i,
    /objects\.githubusercontent\.com/i,
  ];

  const DOWNLOAD_PATH_HINTS = [
    /(?:^|\/)(download|dl|direct|get|fetch|attachment)(?:[/._-]|$)/i,
    /[?&](?:download|attachment|response-content-disposition|response-content-type|filename|file|token|expires|signature|sig)=/i,
  ];

  function resolveUrl(url, baseUrl) {
    if (!url) {
      return null;
    }
    try {
      return baseUrl ? new URL(url, baseUrl) : new URL(url);
    } catch {
      return null;
    }
  }

  function cleanMimeType(value) {
    if (typeof value !== "string") {
      return "";
    }
    return value.split(";")[0].trim().toLowerCase();
  }

  function isViewableMediaExtension(value) {
    return !!value && VIEWABLE_MEDIA_EXTENSIONS.has(String(value).toLowerCase());
  }

  function isViewableMediaMime(value) {
    const mime = cleanMimeType(value);
    return mime.startsWith("image/") || mime.startsWith("video/") || mime.startsWith("audio/");
  }

  function extensionFromUrl(url, baseUrl) {
    const parsed = resolveUrl(url, baseUrl);
    if (!parsed) {
      return "";
    }
    const segments = parsed.pathname.split("/");
    const leaf = segments[segments.length - 1] ?? "";
    const dot = leaf.lastIndexOf(".");
    if (dot === -1) {
      return "";
    }
    return leaf.slice(dot + 1).toLowerCase().split(/[?#&]/)[0];
  }

  function filenameExtension(filename) {
    if (!filename) {
      return "";
    }
    const value = String(filename);
    const dot = value.lastIndexOf(".");
    return dot === -1 ? "" : value.slice(dot + 1).toLowerCase();
  }

  function hintedFilenameFromUrl(url, baseUrl) {
    const parsed = resolveUrl(url, baseUrl);
    if (!parsed) {
      return null;
    }
    for (const key of ["filename", "file", "download", "attachment", "name", "title"]) {
      const value = parsed.searchParams.get(key);
      if (value && value.trim()) {
        return value.trim();
      }
    }
    return null;
  }

  function hasDownloadPathHint(url, baseUrl) {
    const parsed = resolveUrl(url, baseUrl);
    if (!parsed) {
      return false;
    }
    return DOWNLOAD_PATH_HINTS.some(
      (pattern) => pattern.test(parsed.pathname) || pattern.test(parsed.search),
    );
  }

  function isSameOrigin(url, referrerUrl, baseUrl) {
    const parsed = resolveUrl(url, baseUrl);
    const referrer = resolveUrl(referrerUrl);
    return !!parsed && !!referrer && parsed.origin === referrer.origin;
  }

  function isCdnHost(url, baseUrl) {
    const parsed = resolveUrl(url, baseUrl);
    if (!parsed) {
      return false;
    }
    return CDN_PATTERNS.some((pattern) => pattern.test(parsed.hostname));
  }

  function parsePositiveInteger(value) {
    if (value == null || value === "") {
      return null;
    }
    const parsed = Number.parseInt(String(value), 10);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  }

  function decodeMaybeEncoded(value) {
    if (!value) {
      return null;
    }
    try {
      return decodeURIComponent(value);
    } catch {
      return value;
    }
  }

  function parseContentDispositionFilename(value) {
    if (typeof value !== "string" || !value.trim()) {
      return null;
    }
    const filenameStar = value.match(/filename\*\s*=\s*(?:UTF-8''|utf-8'')(?:"?)([^;"]+)/i);
    if (filenameStar) {
      return decodeMaybeEncoded(filenameStar[1]?.trim()) ?? null;
    }
    const quoted = value.match(/filename\s*=\s*"([^"]+)"/i);
    if (quoted) {
      return quoted[1].trim();
    }
    const plain = value.match(/filename\s*=\s*([^;]+)/i);
    if (plain) {
      return plain[1].trim().replace(/^"|"$/g, "");
    }
    return null;
  }

  function normalizeResponseHeaders(responseHeaders) {
    if (!responseHeaders || typeof responseHeaders !== "object") {
      return null;
    }

    const contentType = cleanMimeType(responseHeaders["content-type"]);
    const contentDisposition =
      typeof responseHeaders["content-disposition"] === "string"
        ? responseHeaders["content-disposition"]
        : null;
    const filename = parseContentDispositionFilename(contentDisposition);
    const contentLength = parsePositiveInteger(responseHeaders["content-length"]);
    const acceptRanges =
      typeof responseHeaders["accept-ranges"] === "string"
        ? responseHeaders["accept-ranges"].trim().toLowerCase()
        : null;
    const attachment = /attachment/i.test(contentDisposition ?? "");

    return {
      contentType,
      contentDisposition,
      filename,
      contentLength,
      acceptRanges,
      attachment,
      htmlLike:
        contentType === "text/html" ||
        contentType === "application/xhtml+xml",
    };
  }

  function preferObservedSize(observed, fallbackSize) {
    return observed?.contentLength ?? fallbackSize ?? null;
  }

  function inferMediaIntent(anchor, startNode) {
    let node = startNode instanceof Element ? startNode : null;
    while (node && node !== anchor) {
      if (
        node instanceof HTMLImageElement ||
        node instanceof HTMLPictureElement ||
        node instanceof HTMLVideoElement ||
        node instanceof HTMLAudioElement ||
        node instanceof HTMLSourceElement
      ) {
        return true;
      }
      node = node.parentElement;
    }

    return !!anchor?.querySelector?.("img, picture, video, audio, source");
  }

  function looksLikeFileUrl({
    url,
    baseUrl = null,
    referrerUrl = null,
    explicitDownload = false,
    mediaHint = false,
    filename = null,
  }) {
    const parsed = resolveUrl(url, baseUrl);
    if (!parsed || (parsed.protocol !== "http:" && parsed.protocol !== "https:")) {
      return false;
    }
    if (explicitDownload) {
      return true;
    }

    const ext = extensionFromUrl(parsed.href);
    const hintedFilename = hintedFilenameFromUrl(parsed.href);
    const hintedExt = filenameExtension(hintedFilename);
    const filenameExt = filenameExtension(filename);
    const directUrlHint = hasDownloadPathHint(parsed.href);
    const binaryHost = isCdnHost(parsed.href);
    const strongFileHint =
      (ext && ALWAYS_INTERCEPT_EXTENSIONS.has(ext)) ||
      (hintedExt && ALWAYS_INTERCEPT_EXTENSIONS.has(hintedExt)) ||
      (filenameExt && ALWAYS_INTERCEPT_EXTENSIONS.has(filenameExt));
    const viewableMediaHint =
      isViewableMediaExtension(ext) ||
      isViewableMediaExtension(hintedExt) ||
      isViewableMediaExtension(filenameExt);
    if (!strongFileHint) {
      return false;
    }
    if (viewableMediaHint) {
      if (mediaHint) {
        return false;
      }
      return directUrlHint && binaryHost && !isSameOrigin(parsed.href, referrerUrl, baseUrl);
    }
    if (directUrlHint || mediaHint || binaryHost) {
      return true;
    }
    return !isSameOrigin(parsed.href, referrerUrl, baseUrl) && binaryHost;
  }

  function classifyDownload({
    url,
    mime,
    filename,
    fileSizeBytes,
    settings,
    context = "download-api",
    referrerUrl = null,
    explicitDownload = false,
    mediaHint = false,
    responseHeaders = null,
    baseUrl = null,
  }) {
    const minSizeBytes =
      settings?.minSizeBytes != null && Number.isFinite(settings.minSizeBytes)
        ? Number(settings.minSizeBytes)
        : DEFAULT_MIN_SIZE_BYTES;

    const blockedHosts = (settings?.blockedHosts ?? []).map((value) => String(value).toLowerCase());
    const parsed = resolveUrl(url, baseUrl);
    if (parsed) {
      const host = parsed.hostname.toLowerCase();
      if (blockedHosts.some((pattern) => host.includes(pattern))) {
        return "no";
      }
    }

    const observed = normalizeResponseHeaders(responseHeaders);
    const ext = extensionFromUrl(url, baseUrl);
    const hintedFilename = hintedFilenameFromUrl(url, baseUrl);
    const hintedExt = filenameExtension(hintedFilename);
    const filenameExt = filenameExtension(filename);
    const observedFilenameExt = filenameExtension(observed?.filename);
    const cleanMime = cleanMimeType(observed?.contentType || mime);
    const effectiveSize = preferObservedSize(observed, fileSizeBytes);
    const directUrlHint = hasDownloadPathHint(url, baseUrl);
    const binaryHost = isCdnHost(url, baseUrl);
    const viewableMedia =
      isViewableMediaExtension(observedFilenameExt) ||
      isViewableMediaExtension(ext) ||
      isViewableMediaExtension(filenameExt) ||
      isViewableMediaExtension(hintedExt) ||
      isViewableMediaMime(cleanMime);

    if (context === "link-click") {
      if (explicitDownload) {
        return "yes";
      }
      if (viewableMedia) {
        if (observed?.attachment) {
          return "yes";
        }
        if (!mediaHint && directUrlHint && binaryHost && !isSameOrigin(url, referrerUrl, baseUrl)) {
          return "yes";
        }
        return "no";
      }
      if (
        observedFilenameExt &&
        ALWAYS_INTERCEPT_EXTENSIONS.has(observedFilenameExt) &&
        (observed?.attachment || binaryHost || mediaHint)
      ) {
        return "yes";
      }
      if (filenameExt && ALWAYS_INTERCEPT_EXTENSIONS.has(filenameExt)) {
        return directUrlHint || binaryHost || mediaHint ? "yes" : "no";
      }
      if (hintedExt && ALWAYS_INTERCEPT_EXTENSIONS.has(hintedExt)) {
        return directUrlHint || binaryHost || mediaHint ? "yes" : "no";
      }
      if (ext && ALWAYS_INTERCEPT_EXTENSIONS.has(ext)) {
        if (directUrlHint || mediaHint) {
          return "yes";
        }
        if (binaryHost && !isSameOrigin(url, referrerUrl, baseUrl)) {
          return "yes";
        }
        return "no";
      }
      return binaryHost && (directUrlHint || mediaHint) ? "yes" : "no";
    }

    if (viewableMedia && !observed?.attachment) {
      return "no";
    }

    if (observed?.htmlLike) {
      const strongObservedName =
        (observedFilenameExt && ALWAYS_INTERCEPT_EXTENSIONS.has(observedFilenameExt)) ||
        (filenameExt && ALWAYS_INTERCEPT_EXTENSIONS.has(filenameExt)) ||
        (hintedExt && ALWAYS_INTERCEPT_EXTENSIONS.has(hintedExt));
      if (!observed.attachment && !strongObservedName) {
        return "no";
      }
    }

    if (observedFilenameExt && ALWAYS_INTERCEPT_EXTENSIONS.has(observedFilenameExt)) {
      return "yes";
    }
    if (ext && ALWAYS_INTERCEPT_EXTENSIONS.has(ext)) return "yes";
    if (filenameExt && ALWAYS_INTERCEPT_EXTENSIONS.has(filenameExt)) return "yes";
    if (hintedExt && ALWAYS_INTERCEPT_EXTENSIONS.has(hintedExt)) return "yes";

    if (ALWAYS_INTERCEPT_MIMES.has(cleanMime)) {
      if (cleanMime === "application/octet-stream") {
        if (effectiveSize != null && effectiveSize >= minSizeBytes) return "yes";
        if (effectiveSize == null) return observed?.attachment ? "yes" : "maybe";
        return "no";
      }
      return "yes";
    }

    if (observed?.attachment) {
      if (effectiveSize != null && effectiveSize >= minSizeBytes) {
        return "yes";
      }
      if (observed.filename) {
        return "yes";
      }
      return "maybe";
    }

    if (binaryHost) {
      if (effectiveSize != null && effectiveSize >= minSizeBytes) return "yes";
      return "maybe";
    }

    return "no";
  }

  globalScope.VDMDownloadRules = {
    ALWAYS_INTERCEPT_EXTENSIONS,
    ALWAYS_INTERCEPT_MIMES,
    CDN_PATTERNS,
    DEFAULT_MIN_SIZE_BYTES,
    classifyDownload,
    cleanMimeType,
    extensionFromUrl,
    filenameExtension,
    hasDownloadPathHint,
    hintedFilenameFromUrl,
    inferMediaIntent,
    isCdnHost,
    isViewableMediaExtension,
    isViewableMediaMime,
    looksLikeFileUrl,
    normalizeResponseHeaders,
    parseContentDispositionFilename,
  };
})(globalThis);