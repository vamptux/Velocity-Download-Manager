/**
 * VDM Catcher — Content Script (interceptor.js)
 *
 * Runs in the context of every HTTP/HTTPS page.
 * Intercepts anchor-tag clicks that point to downloadable resources
 * before the browser navigates or triggers a download, sends the URL
 * to the service worker, and optionally prevents the default action.
 */

const rules = globalThis.VDMDownloadRules;

function inferMediaIntent(anchor, startNode) {
  if (!rules?.inferMediaIntent) {
    return false;
  }
  return rules.inferMediaIntent(anchor, startNode);
}

function getFilenameFromAnchor(anchor) {
  if (anchor.download) return anchor.download;
  try {
    const path = new URL(anchor.href, document.baseURI).pathname;
    const segment = path.split("/").filter(Boolean).pop();
    return segment && segment.includes(".") ? decodeURIComponent(segment) : null;
  } catch {
    return null;
  }
}

function looksLikeFileUrl(url, anchor, mediaHint) {
  if (!rules?.looksLikeFileUrl) {
    return false;
  }
  return rules.looksLikeFileUrl({
    url,
    baseUrl: document.baseURI,
    referrerUrl: document.URL,
    explicitDownload: anchor.hasAttribute("download"),
    mediaHint,
    filename: getFilenameFromAnchor(anchor),
  });
}

function navigateFallback(url, openInNewTab) {
  if (openInNewTab) {
    window.open(url, "_blank", "noopener,noreferrer");
  } else {
    window.location.assign(url);
  }
}

async function captureAnchorClick(anchor, openInNewTab) {
  const url = new URL(anchor.href, document.baseURI).href;
  const filename = getFilenameFromAnchor(anchor);
  const mediaHint = inferMediaIntent(anchor, anchor);

  let response;
  try {
    response = await chrome.runtime.sendMessage({
      type: "intercept-link",
      url,
      referrer: document.URL,
      filename,
      downloadAttribute: anchor.hasAttribute("download"),
      mediaHint,
    });
  } catch {
    return;
  }

  if (!response?.intercepted || !response?.shouldPrevent) {
    navigateFallback(url, openInNewTab);
  }
}

function findAnchor(startNode) {
  let node = startNode;
  while (node && node !== document) {
    if (node instanceof HTMLAnchorElement && node.href) {
      return node;
    }
    node = node.parentElement;
  }
  return null;
}

function onAnchorEvent(event) {
  if (event.defaultPrevented) return;
  if (event instanceof MouseEvent) {
    if (event.button !== 0 && event.button !== 1) return;
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  }

  const anchor = findAnchor(event.target);
  if (!anchor) return;

  const mediaHint = inferMediaIntent(anchor, event.target);
  if (looksLikeFileUrl(anchor.href, anchor, mediaHint)) {
    const openInNewTab =
      anchor.target === "_blank" ||
      (event instanceof MouseEvent && event.button === 1);
    event.preventDefault();
    event.stopImmediatePropagation();
    void captureAnchorClick(anchor, openInNewTab);
  }
}

document.addEventListener("click", onAnchorEvent, true);
document.addEventListener("auxclick", onAnchorEvent, true);