// Synthesis scraper — standalone JS expression evaluated in the browser.
//
// Template markers replaced at runtime by Rust:
//   __LIMIT__            → max number of articles to return
//   __SOURCE__           → source label string (pre-escaped)
//   __TIMEOUT_BUDGET_MS__ → max runtime in ms before graceful degradation (default 12000)
//
// This file is included in the binary via include_str!("mcp/scraper.js")
// and is a single IIFE that returns a JSON string.

(async () => {
    const results = [];
    const seenTitles = new Set();
    const seenUrls = new Set();
    const LIMIT = __LIMIT__;
    
    const MIN_TITLE_LEN = 25;
    const BUDGET_MS = __TIMEOUT_BUDGET_MS__;
    const DEADLINE = Date.now() + BUDGET_MS;

    const NAV_REJECT = /^(browse|watch|streaming|subscribe|sign in|log in|sign up|about|contact|faq|help|terms|privacy|cookie|advertise|feedback|more|menu|search|home|back to|go to|view all|see all|read more|load more|show more|expand|collapse|close|dismiss|skip|next|prev|previous|comments?\b|share|reply|report)/i;
    const JUNK_TITLE = /^(comment|share|reply|report|bookmark|save|like|upvote|downvote)\s*(icon)?/i;

    const MEDIA_URL_REJECT = /\.(mp3|mp4|wav|ogg|m4a|aac|flac|webm|avi|mkv|pdf|zip|tar|gz|exe|dmg|deb|rpm)\b/i;

    // Known ad / tracking network domains — images served from these are never editorial.
    const AD_DOMAINS = /doubleclick\.net|googlesyndication\.com|googleadservices\.com|amazon-adsystem\.com|adsystem\.com|adservice\.|taboola\.com|outbrain\.com|criteo\.(com|net)|moat\.com|advertising\.com|adnxs\.com|adsrvr\.org|pubmatic\.com|rubiconproject\.com|openx\.net|casalemedia\.com|indexexchange\.com|contextweb\.com|spotxchange\.com|33across\.com|sharethrough\.com|liveintent\.com|media\.net\b|mgid\.com|revcontent\.com|zergnet\.com|content-ad\.net/i;

    // CSS selectors for ad container elements
    const AD_CONTAINER_SELECTORS = [
        '[class*="ad-"]', '[class*="advert"]', '[class*="ad_"]',
        '[id*="ad-"]', '[id*="advert"]', '[id*="ad_"]',
        '[class*="sponsor"]', '[class*="promoted"]', '[class*="paid-"]',
        '[data-ad]', '[data-advertisement]', '[data-ad-slot]',
        '[data-google-query-id]', '[aria-label*="advertisement"]',
        '[aria-label*="Advertisement"]', '[aria-label*="Sponsored"]',
        'ins.adsbygoogle', '.ad-container', '.ad-wrapper',
        '[id^="google_ads"]', '[id^="div-gpt-ad"]'
    ].join(', ');

    function isAdElement(el) {
        if (!el) return false;
        let node = el;
        while (node && node !== document.body) {
            if (node.matches && node.matches(AD_CONTAINER_SELECTORS)) return true;
            // Check if inside <aside> but NOT inside <article>
            const tag = node.tagName?.toLowerCase();
            if (tag === 'aside' && !node.closest('article')) return true;
            node = node.parentElement;
        }
        return false;
    }

    function isAdImageUrl(u) {
        if (!u) return false;
        try { return AD_DOMAINS.test(new URL(u).hostname); } catch { return false; }
    }

    function addResult(title, url, image, description, locked) {
        title = cleanTitle(title);
        if (!title || title.length < MIN_TITLE_LEN) return false;
        if (NAV_REJECT.test(title) || JUNK_TITLE.test(title)) return false;
        url = normalizeUrl(url);
        if (!url) return false;
        if (MEDIA_URL_REJECT.test(url)) return false;
        const dedupUrl = url.replace(/#.*$/, '');
        if (seenTitles.has(title) || seenUrls.has(dedupUrl)) return false;
        seenTitles.add(title);
        seenUrls.add(dedupUrl);
        const cleanDesc = (description || '')
            .replace(/\s+/g, ' ')
            .replace(/\s*[A-Z][a-z]+\s+[A-Z][a-z]+\/(AP|AFP|Reuters|Getty|EPA)\s*$/i, '')
            .trim()
            .substring(0, 300);
        results.push({
            title,
            url: dedupUrl,
            image: image || '',
            description: cleanDesc,
            _locked: !!locked
        });
        return true;
    }

    function normalizeUrl(u) {
        try { return new URL(u, location.href).href; } catch { return u || ''; }
    }

    function imageKey(u) {
        try { const p = new URL(u); return p.origin + p.pathname; }
        catch { return u || ''; }
    }

    function cleanTitle(t) {
        if (!t) return '';
        return t
            .replace(/\s+/g, ' ')
            .replace(/^\s*[•·▸►▶|]\s*/g, '')
            .replace(/^\s*(Video|LIVE|BREAKING|NEW|WATCH|EXCLUSIVE|OPINION|ANALYSIS)\s*[:\|]?\s*/gi, '')
            .replace(/\b\d+:\d{2}\b/g, '')
            .replace(/\s*(social media|advertisement|sponsored)\s*/gi, '')
            .replace(/\s*\|\s*$/, '')
            .trim();
    }

    function isInsideNav(el) {
        let node = el;
        while (node && node !== document.body) {
            const tag = node.tagName?.toLowerCase();
            if (tag === 'nav' || tag === 'footer' || tag === 'aside') return true;
            if (tag === 'header') {
                if (node.closest('article')) return false;
                return true;
            }
            const cls = (node.className || '').toString().toLowerCase();
            if (/\b(nav|menu|footer|sidebar|breadcrumb|toolbar|skip-link)\b/.test(cls)) return true;
            const role = (node.getAttribute?.('role') || '').toLowerCase();
            if (role === 'navigation' || role === 'banner' || role === 'contentinfo') return true;
            node = node.parentElement;
        }
        return false;
    }

    function isValidImageUrl(u) {
        if (!u || u.length < 10) return false;
        if (u.startsWith('data:')) return false;
        if (u.endsWith('.svg') || u.includes('/icons/') || u.includes('/icon/')) return false;
        if (/1x1|pixel|spacer|blank|tracking|beacon/i.test(u)) return false;
        if (isAdImageUrl(u)) return false;
        return true;
    }

    function findBestImage(container) {
        if (!container) return '';
        if (isAdElement(container)) return '';
        const imgs = container.querySelectorAll('img');
        for (const img of imgs) {
            if (isAdElement(img)) continue;
            const src = img.getAttribute('src');
            if (isValidImageUrl(src)) return normalizeUrl(src);
            for (const attr of ['data-src', 'data-lazy-src', 'data-original', 'data-image-src', 'data-uri']) {
                const v = img.getAttribute(attr);
                if (isValidImageUrl(v)) return normalizeUrl(v);
            }
            const srcset = img.getAttribute('srcset');
            if (srcset) {
                const first = srcset.split(',')[0].trim().split(/\s/)[0];
                if (isValidImageUrl(first)) return normalizeUrl(first);
            }
        }
        const picSrc = container.querySelector('picture source[srcset]');
        if (picSrc && !isAdElement(picSrc)) {
            const first = picSrc.getAttribute('srcset').split(',')[0].trim().split(/\s/)[0];
            if (isValidImageUrl(first)) return normalizeUrl(first);
        }
        const noscript = container.querySelector('noscript');
        if (noscript) {
            const m = noscript.textContent.match(/src=["']([^"']+)["']/);
            if (m && isValidImageUrl(m[1])) return normalizeUrl(m[1]);
        }
        const bgCandidates = [container, ...container.querySelectorAll('[style*="background"]')];
        for (const el of bgCandidates) {
            if (isAdElement(el)) continue;
            const style = el.getAttribute('style') || '';
            const m = style.match(/background-image\s*:\s*url\(\s*['"]?([^'")]+)['"]?\s*\)/);
            if (m && isValidImageUrl(m[1])) return normalizeUrl(m[1]);
        }
        const bgDivs = container.querySelectorAll('div, span, figure');
        for (const el of bgDivs) {
            if (isAdElement(el)) continue;
            try {
                const bg = getComputedStyle(el).backgroundImage;
                if (bg && bg !== 'none') {
                    const m = bg.match(/url\(\s*["']?([^"')]+)["']?\s*\)/);
                    if (m && isValidImageUrl(m[1])) return normalizeUrl(m[1]);
                }
            } catch(e) {}
        }
        const dataBg = container.querySelector('[data-bg]');
        if (dataBg && !isAdElement(dataBg)) {
            const v = dataBg.getAttribute('data-bg');
            if (isValidImageUrl(v)) return normalizeUrl(v);
        }
        return '';
    }

    const DESC_REJECT = /^(preview|advertisement|sponsored|loading|please wait|sign up|subscribe|cookie|accept)/i;

    function findBestDescription(container, headingEl) {
        if (!container) return '';

        const headingCount = container.querySelectorAll('h1, h2, h3, h4').length;
        if (headingCount > 3) return '';

        const selectors = ['[class*="summary"]', '[class*="description"]', '[class*="dek"]', '[class*="excerpt"]', '[class*="synopsis"]', '[class*="standfirst"]', '[class*="lede"]', '[class*="subhead"]', '[class*="blurb"]', 'p'];
        for (const sel of selectors) {
            const el = container.querySelector(sel);
            if (el) {
                const t = el.textContent?.trim();
                if (t && t.length > 20 && t.length < 500 && !DESC_REJECT.test(t)) {
                    if (headingEl && headingCount > 1) {
                        const hParent = headingEl.parentElement;
                        const pParent = el.parentElement;
                        if (hParent && pParent && hParent !== pParent) {
                            const hContainer = headingEl.closest('[class]');
                            const pContainer = el.closest('[class]');
                            if (hContainer && pContainer && hContainer !== pContainer) continue;
                        }
                    }
                    return t;
                }
            }
        }
        const ariaLabel = container.getAttribute?.('aria-label');
        if (ariaLabel && ariaLabel.length > 30 && !DESC_REJECT.test(ariaLabel)) return ariaLabel.substring(0, 300);
        return '';
    }

    function findContainer(el) {
        return el?.closest('article, [class*="card"], [class*="story"], [class*="promo"], [class*="teaser"], [class*="media-block"], [class*="content-card"], [class*="feed-item"], [class*="river-item"], [role="article"], li[class]') || el?.parentElement?.parentElement;
    }

    // =================================================================
    // Strategy 0: JSON-LD structured data (highest quality — locked)
    // =================================================================
    document.querySelectorAll('script[type="application/ld+json"]').forEach(script => {
        if (results.length >= LIMIT) return;
        try {
            let data = JSON.parse(script.textContent);
            if (data['@graph']) data = data['@graph'];
            const items = Array.isArray(data) ? data : [data];
            for (const item of items) {
                if (results.length >= LIMIT) break;
                const type = item['@type'];
                if (!type) continue;
                const types = Array.isArray(type) ? type : [type];
                const isArticle = types.some(t => /Article|NewsArticle|BlogPosting|ReportageNewsArticle|LiveBlogPosting/i.test(t));
                if (!isArticle) continue;
                const title = item.headline || item.name || '';
                const url = item.url || item.mainEntityOfPage?.['@id'] || item.mainEntityOfPage || '';
                let image = '';
                if (item.image) {
                    if (typeof item.image === 'string') image = item.image;
                    else if (Array.isArray(item.image)) image = typeof item.image[0] === 'string' ? item.image[0] : item.image[0]?.url || '';
                    else image = item.image.url || item.image.contentUrl || '';
                } else if (item.thumbnailUrl) {
                    image = item.thumbnailUrl;
                }
                const desc = item.description || item.abstract || '';
                // JSON-LD images are authoritative — mark locked
                addResult(title, url, image, desc, /* locked */ !!image);
            }
        } catch (e) {}
    });

    // === Strategy 1: <article> elements with headings (skip nav/footer) ===
    if (results.length < LIMIT) {
        document.querySelectorAll('article').forEach(article => {
            if (results.length >= LIMIT) return;
            if (isInsideNav(article)) return;
            if (isAdElement(article)) return;
            const heading = article.querySelector('h1, h2, h3, h4');
            const link = article.querySelector('a[href]') || heading?.closest('a');
            const image = findBestImage(article);
            const desc = findBestDescription(article, heading);
            if (heading) {
                addResult(heading.textContent, link?.href || '', image, desc);
            }
        });
    }

    // === Strategy 2: heading+link combos (skip nav/footer) ===
    if (results.length < LIMIT) {
        document.querySelectorAll('h2 a[href], h3 a[href], a h2, a h3').forEach(el => {
            if (results.length >= LIMIT) return;
            if (isInsideNav(el)) return;
            if (isAdElement(el)) return;
            const a = el.tagName === 'A' ? el : el.closest('a');
            const heading = el.tagName.match(/^H[2-3]$/) ? el : el.querySelector('h2, h3');
            const text = heading?.textContent || a?.textContent || '';
            const container = findContainer(a);
            const image = findBestImage(container);
            const desc = findBestDescription(container, heading);
            addResult(text, a?.href || '', image, desc);
        });
    }

    // === Strategy 3: Card/link patterns (data attrs, class selectors) ===
    if (results.length < LIMIT) {
        const cardSelectors = 'a[data-vars-item-name], a[class*="card"], a[class*="story"], a[class*="headline"], a[class*="container__link"], a[class*="promo"], [class*="card"] a[href], [class*="story"] a[href], [class*="promo"] a[href]';
        document.querySelectorAll(cardSelectors).forEach(el => {
            if (results.length >= LIMIT) return;
            if (isInsideNav(el)) return;
            if (isAdElement(el)) return;
            const a = el.tagName === 'A' ? el : el.querySelector('a[href]') || el.closest('a');
            if (!a) return;
            const name = a.getAttribute('data-vars-item-name') || '';
            const heading = el.querySelector('h2, h3, h4, [class*="headline"], [class*="title"]');
            const text = name || heading?.textContent || '';
            if (!text) return;
            const container = findContainer(a) || a;
            const image = findBestImage(container);
            const desc = findBestDescription(container, heading);
            addResult(text, a.href || '', image, desc);
        });
    }

    // === Strategy 4: <li> with headings or images (require substance, skip nav) ===
    if (results.length < LIMIT) {
        document.querySelectorAll('ul li, ol li').forEach(li => {
            if (results.length >= LIMIT) return;
            if (isInsideNav(li)) return;
            if (isAdElement(li)) return;
            const heading = li.querySelector('h2, h3, h4, [class*="headline"], [class*="title"]');
            const a = li.querySelector('a[href]');
            const image = findBestImage(li);
            if (!heading && !image) return;
            const text = heading?.textContent || a?.textContent || '';
            const desc = findBestDescription(li, heading);
            addResult(text, a?.href || '', image, desc);
        });
    }

    // === Strategy 5: [role="article"] or semantic containers ===
    if (results.length < LIMIT) {
        document.querySelectorAll('[role="article"], [itemtype*="schema.org/Article"], [data-type="article"]').forEach(el => {
            if (results.length >= LIMIT) return;
            if (isInsideNav(el)) return;
            if (isAdElement(el)) return;
            const heading = el.querySelector('h1, h2, h3, h4, [class*="headline"]');
            const link = el.querySelector('a[href]') || heading?.closest('a');
            if (!heading) return;
            const image = findBestImage(el);
            const desc = findBestDescription(el, heading);
            addResult(heading.textContent, link?.href || '', image, desc);
        });
    }

    // === Strategy 6: Table-row / repeated-class content items ===
    if (results.length < LIMIT) {
        document.querySelectorAll('tr[class] a[href], tr[id] a[href]').forEach(a => {
            if (results.length >= LIMIT) return;
            if (isInsideNav(a)) return;
            const text = a.textContent?.trim();
            if (!text || text.length < 10) return;
            const href = a.href;
            if (!href || href === location.href || href.startsWith('javascript:')) return;
            const row = a.closest('tr');
            const image = row ? findBestImage(row) : '';
            const desc = '';
            addResult(text, href, image, desc);
        });
    }

    // === Strategy 7: Prominent standalone links (final fallback) ===
    if (results.length < LIMIT) {
        const allLinks = Array.from(document.querySelectorAll('a[href]'));
        const candidates = allLinks.filter(a => {
            if (isInsideNav(a)) return false;
            if (isAdElement(a)) return false;
            const text = a.textContent?.trim();
            if (!text || text.length < 15 || text.length > 200) return false;
            const href = a.href;
            if (!href || href === location.href) return false;
            if (href.startsWith('javascript:') || href.startsWith('mailto:')) return false;
            if (!text.includes(' ') && text.length < 30) return false;
            return true;
        });
        for (const a of candidates) {
            if (results.length >= LIMIT) break;
            const container = findContainer(a) || a.parentElement;
            const image = container ? findBestImage(container) : '';
            const desc = container ? findBestDescription(container) : '';
            addResult(a.textContent?.trim(), a.href, image, desc);
        }
    }

    // === Strategy 8: OpenGraph / meta fallback (single-article pages) ===
    if (results.length === 0) {
        const ogTitle = document.querySelector('meta[property="og:title"]')?.content;
        const ogDesc = document.querySelector('meta[property="og:description"]')?.content;
        const ogImage = document.querySelector('meta[property="og:image"]')?.content;
        const ogUrl = document.querySelector('meta[property="og:url"]')?.content;
        if (ogTitle) {
            addResult(ogTitle, ogUrl || window.location.href, ogImage || '', ogDesc || '');
        }
    }

    // ================================================================
    // UNIVERSAL LINK-ASSOCIATION ENGINE
    // ================================================================

    function extractImagesFromElement(el) {
        const imgs = [];
        // 1. <img> elements
        el.querySelectorAll('img').forEach(img => {
            if (isAdElement(img)) return;
            const src = img.currentSrc || img.src || img.dataset?.src
                || img.dataset?.lazySrc || img.dataset?.original || '';
            if (!isValidImageUrl(src)) return;
            const w = img.naturalWidth || parseInt(img.getAttribute('width')) || 0;
            const h = img.naturalHeight || parseInt(img.getAttribute('height')) || 0;
            const rect = img.getBoundingClientRect();
            const rendered = rect.width * rect.height;
            const natural = w * h;
            imgs.push({ src: normalizeUrl(src), score: Math.max(rendered, natural), loaded: img.complete && w > 0 });
            const srcset = img.getAttribute('srcset');
            if (srcset) {
                const parts = srcset.split(',').map(s => s.trim().split(/\s+/));
                for (const p of parts) {
                    if (p[0] && isValidImageUrl(p[0]) && p[0] !== src) {
                        const sz = parseInt(p[1]) || 0;
                        imgs.push({ src: normalizeUrl(p[0]), score: sz * sz || rendered, loaded: true });
                    }
                }
            }
        });
        // 2. <picture> > <source>
        el.querySelectorAll('picture source[srcset]').forEach(src => {
            if (isAdElement(src)) return;
            const srcset = src.getAttribute('srcset');
            const parts = srcset.split(',').map(s => s.trim().split(/\s+/));
            for (const p of parts) {
                if (p[0] && isValidImageUrl(p[0])) {
                    const sz = parseInt(p[1]) || 100;
                    imgs.push({ src: normalizeUrl(p[0]), score: sz * sz, loaded: true });
                }
            }
        });
        // 3. CSS background-image
        const bgCandidates = [el];
        el.querySelectorAll('div, span, figure').forEach(c => bgCandidates.push(c));
        for (const c of bgCandidates) {
            if (isAdElement(c)) continue;
            try {
                const bg = getComputedStyle(c).backgroundImage;
                if (!bg || bg === 'none') continue;
                const m = bg.match(/url\(\s*["']?([^"')]+)["']?\s*\)/);
                if (m && isValidImageUrl(m[1])) {
                    const rect = c.getBoundingClientRect();
                    imgs.push({ src: normalizeUrl(m[1]), score: rect.width * rect.height, loaded: true });
                }
            } catch(e) {}
        }
        // 4. data-bg, data-background attributes
        el.querySelectorAll('[data-bg], [data-background]').forEach(c => {
            if (isAdElement(c)) return;
            const v = c.getAttribute('data-bg') || c.getAttribute('data-background') || '';
            if (isValidImageUrl(v)) {
                const rect = c.getBoundingClientRect();
                imgs.push({ src: normalizeUrl(v), score: rect.width * rect.height, loaded: true });
            }
        });
        // 5. <noscript> img fallback
        el.querySelectorAll('noscript').forEach(ns => {
            const m = ns.textContent?.match(/src=["']([^"']+)["']/);
            if (m && isValidImageUrl(m[1])) {
                imgs.push({ src: normalizeUrl(m[1]), score: 100, loaded: true });
            }
        });
        // 6. <video poster>
        el.querySelectorAll('video[poster]').forEach(v => {
            if (isAdElement(v)) return;
            const poster = v.getAttribute('poster');
            if (isValidImageUrl(poster)) {
                const rect = v.getBoundingClientRect();
                imgs.push({ src: normalizeUrl(poster), score: rect.width * rect.height || 10000, loaded: true });
            }
        });
        // 7. <video> source URL thumbnail derivation
        el.querySelectorAll('video source[src]').forEach(s => {
            const src = s.src || '';
            const thumbUrl = src.replace(/\.(mp4|webm|ogg)(\?.*)?$/i, '.jpg');
            if (thumbUrl !== src && isValidImageUrl(thumbUrl)) {
                const rect = s.closest('video')?.getBoundingClientRect() || { width: 0, height: 0 };
                imgs.push({ src: normalizeUrl(thumbUrl), score: rect.width * rect.height || 100, loaded: false });
            }
        });
        return imgs;
    }

    // === Build URL -> Assets map from every <a> on the page ===
    const urlAssets = new Map();
    document.querySelectorAll('a[href]').forEach(a => {
        const href = a.href;
        if (!href || href === location.href || href.startsWith('javascript:') || href.startsWith('mailto:')) return;
        if (isAdElement(a)) return;

        if (!urlAssets.has(href)) urlAssets.set(href, { images: [], descs: [] });
        const assets = urlAssets.get(href);

        const linkImages = extractImagesFromElement(a);
        assets.images.push(...linkImages);

        const container = findContainer(a) || a.parentElement;
        if (container) {
            const desc = findBestDescription(container);
            if (desc && desc.length > 20) assets.descs.push(desc);
        }
    });

    // Minimum rendered area for container-recovery images (Phase 2).
    // Images from link-association (Phase 1) skip this check since
    // an image inside the same <a> tag is almost certainly correct.
    const MIN_IMAGE_SCORE = 3000;

    // === Phase 1: Link-Association (PRIMARY image source) ===
    const usedKeys = new Set();

    results.forEach(r => {
        if (r._locked && r.image) {
            usedKeys.add(imageKey(r.image));
            return;
        }
        if (!r.url) return;
        const assets = urlAssets.get(r.url);
        if (!assets || assets.images.length === 0) {
            if (r.image) usedKeys.add(imageKey(r.image));
            return;
        }

        const sorted = [...assets.images]
            .sort((a, b) => (b.loaded ? 1 : 0) - (a.loaded ? 1 : 0) || b.score - a.score);
        const best = sorted.find(i => !usedKeys.has(imageKey(i.src)));
        if (best) {
            r.image = best.src;
            usedKeys.add(imageKey(best.src));
        } else if (sorted.length > 0 && !r.image) {
            r.image = sorted[0].src;
        }
    });

    // === Phase 2: Container image recovery (reduced walk) ===
    // Walk up at most 3 levels from each article's title link.
    // Only accept images from semantic containers, not arbitrary divs.
    const SEMANTIC_CONTAINER = /^(article|section|li)$/i;
    const SEMANTIC_CLASS = /card|story|promo|teaser|media-block|content-card|feed-item|river-item/i;

    results.forEach(r => {
        if (r._locked || r.image || !r.url) return;
        try {
            const titleLinks = [];
            document.querySelectorAll('a[href]').forEach(a => {
                if (a.href === r.url) titleLinks.push(a);
            });

            for (const tl of titleLinks) {
                const tlRect = tl.getBoundingClientRect();
                const tx = tlRect.left + tlRect.width / 2;
                const ty = tlRect.top + tlRect.height / 2;

                let container = tl.parentElement;
                for (let depth = 0; depth < 3 && container; depth++) {
                    const tag = container.tagName?.toLowerCase() || '';
                    const cls = (container.className || '').toString();
                    const isSemantic = SEMANTIC_CONTAINER.test(tag) || SEMANTIC_CLASS.test(cls) || container.getAttribute('role') === 'article';

                    if (!isSemantic) {
                        container = container.parentElement;
                        continue;
                    }

                    if (isAdElement(container)) {
                        container = container.parentElement;
                        continue;
                    }

                    const imgs = extractImagesFromElement(container);
                    const available = imgs.filter(i =>
                        !usedKeys.has(imageKey(i.src)) && i.score >= MIN_IMAGE_SCORE
                    );
                    if (available.length > 0) {
                        for (const img of available) {
                            let imgEl = null;
                            container.querySelectorAll('img, video, [style*="background"]').forEach(el => {
                                const src = el.currentSrc || el.src || el.getAttribute('poster') || '';
                                if (normalizeUrl(src) === img.src || imageKey(src) === imageKey(img.src)) imgEl = el;
                            });
                            if (imgEl) {
                                const ir = imgEl.getBoundingClientRect();
                                const dx = (ir.left + ir.width/2) - tx;
                                const dy = (ir.top + ir.height/2) - ty;
                                img.dist = Math.sqrt(dx*dx + dy*dy);
                            } else {
                                img.dist = 9999;
                            }
                        }
                        available.sort((a, b) => a.dist - b.dist || (b.loaded ? 1 : 0) - (a.loaded ? 1 : 0) || b.score - a.score);
                        r.image = available[0].src;
                        usedKeys.add(imageKey(available[0].src));
                        break;
                    }
                    container = container.parentElement;
                }
                if (r.image) break;
            }
        } catch(e) {}
    });

    // === Phase 3: Video spatial correlation ===
    // Find <video> elements on the page and match them to articles by
    // proximity to the article's title link.  When a match is found,
    // try to derive a usable static image URL (poster, source thumbnail).
    // If none is available, flag the article for og:image recovery.
    results.forEach(r => {
        if (r._locked || r.image || !r.url) return;
        try {
            const titleLinks = [];
            document.querySelectorAll('a[href]').forEach(a => {
                if (a.href === r.url) titleLinks.push(a);
            });
            if (titleLinks.length === 0) return;

            const allVideos = document.querySelectorAll('video');
            if (allVideos.length === 0) return;

            for (const tl of titleLinks) {
                const tlRect = tl.getBoundingClientRect();
                const tx = tlRect.left + tlRect.width / 2;
                const ty = tlRect.top + tlRect.height / 2;

                let bestVideo = null;
                let bestDist = Infinity;

                allVideos.forEach(video => {
                    if (isAdElement(video)) return;
                    const vr = video.getBoundingClientRect();
                    if (vr.width < 40 || vr.height < 30) return;
                    const vx = vr.left + vr.width / 2;
                    const vy = vr.top + vr.height / 2;
                    const dist = Math.sqrt((vx - tx) ** 2 + (vy - ty) ** 2);
                    if (dist < 500 && dist < bestDist) {
                        bestDist = dist;
                        bestVideo = video;
                    }
                });

                if (!bestVideo) continue;

                const poster = bestVideo.getAttribute('poster');
                if (poster && isValidImageUrl(poster)) {
                    const k = imageKey(normalizeUrl(poster));
                    if (!usedKeys.has(k)) {
                        r.image = normalizeUrl(poster);
                        usedKeys.add(k);
                        break;
                    }
                }

                const sources = bestVideo.querySelectorAll('source[src]');
                for (const s of sources) {
                    const src = s.src || s.getAttribute('src') || '';
                    const thumbUrl = src.replace(/\.(mp4|webm|ogg)(\?.*)?$/i, '.jpg');
                    if (thumbUrl !== src && isValidImageUrl(thumbUrl)) {
                        const k = imageKey(normalizeUrl(thumbUrl));
                        if (!usedKeys.has(k)) {
                            r.image = normalizeUrl(thumbUrl);
                            usedKeys.add(k);
                            break;
                        }
                    }
                }
                if (r.image) break;

                const videoSrc = bestVideo.src || bestVideo.currentSrc || '';
                if (videoSrc) {
                    const thumbUrl = videoSrc.replace(/\.(mp4|webm|ogg)(\?.*)?$/i, '.jpg');
                    if (thumbUrl !== videoSrc && isValidImageUrl(thumbUrl)) {
                        const k = imageKey(normalizeUrl(thumbUrl));
                        if (!usedKeys.has(k)) {
                            r.image = normalizeUrl(thumbUrl);
                            usedKeys.add(k);
                            break;
                        }
                    }
                }

                r._hasNearbyVideo = true;
            }
        } catch(e) {}
    });

    // === Phase 4: og:image fallback for missing images ===
    // Budget-aware: skip if we've used too much time already.
    // Cap parallel fetches at 5 to avoid network flooding.
    const missingImage = results.filter(r => !r.image && r.url);
    let ogSkipped = 0;
    if (missingImage.length > 0 && Date.now() < DEADLINE - 2000) {
        const OG_TIMEOUT = 2000;
        const MAX_PARALLEL = 5;
        const batch = missingImage.slice(0, MAX_PARALLEL);
        ogSkipped = Math.max(0, missingImage.length - MAX_PARALLEL);
        const fetchOg = async (article) => {
            if (Date.now() >= DEADLINE - 1000) { ogSkipped++; return; }
            try {
                const controller = new AbortController();
                const timer = setTimeout(() => controller.abort(), OG_TIMEOUT);
                const resp = await fetch(article.url, {
                    signal: controller.signal,
                    credentials: 'omit',
                    headers: { 'Accept': 'text/html' }
                });
                clearTimeout(timer);
                if (!resp.ok) return;
                const html = await resp.text();
                const patterns = [
                    /<meta[^>]+property=["']og:image["'][^>]+content=["']([^"']+)["']/i,
                    /<meta[^>]+content=["']([^"']+)["'][^>]+property=["']og:image["']/i,
                    /<meta[^>]+name=["']twitter:image["'][^>]+content=["']([^"']+)["']/i,
                    /<meta[^>]+content=["']([^"']+)["'][^>]+name=["']twitter:image["']/i
                ];
                for (const pat of patterns) {
                    const m = html.match(pat);
                    if (m && isValidImageUrl(m[1])) {
                        const imgUrl = normalizeUrl(m[1]);
                        const k = imageKey(imgUrl);
                        if (!usedKeys.has(k)) {
                            article.image = imgUrl;
                            article._ogRecovered = true;
                            usedKeys.add(k);
                        }
                        return;
                    }
                }
                // Also extract og:description as fallback
                const descPat = [
                    /<meta[^>]+property=["']og:description["'][^>]+content=["']([^"']+)["']/i,
                    /<meta[^>]+content=["']([^"']+)["'][^>]+property=["']og:description["']/i,
                    /<meta[^>]+name=["']description["'][^>]+content=["']([^"']+)["']/i,
                    /<meta[^>]+content=["']([^"']+)["'][^>]+name=["']description["']/i
                ];
                for (const pat of descPat) {
                    const m = html.match(pat);
                    if (m && m[1].length > 20) {
                        article._ogDescription = m[1].substring(0, 300);
                        return;
                    }
                }
            } catch(e) {}
        };
        await Promise.allSettled(batch.map(a => fetchOg(a)));
    } else if (missingImage.length > 0) {
        ogSkipped = missingImage.length;
    }

    // === Phase 5: Description recovery ===
    const usedDescs = new Set();
    results.forEach(r => {
        if (r.description) {
            usedDescs.add(r.description);
            return;
        }
    });

    results.forEach(r => {
        if (r.description) return;

        if (r.url) {
            const assets = urlAssets.get(r.url);
            if (assets && assets.descs.length > 0) {
                const best = assets.descs
                    .filter(d => d !== r.title && d.length > 20 && !usedDescs.has(d))
                    .sort((a, b) => b.length - a.length)[0];
                if (best) {
                    r.description = best.substring(0, 300);
                    usedDescs.add(r.description);
                    return;
                }
            }
        }

        if (!r.description && r.url) {
            try {
                const links = [];
                document.querySelectorAll('a[href]').forEach(a => {
                    if (a.href === r.url) links.push(a);
                });
                for (const link of links) {
                    const container = findContainer(link);
                    if (container) {
                        const desc = findBestDescription(container);
                        if (desc && desc !== r.title && desc.length > 20 && !usedDescs.has(desc)) {
                            r.description = desc.substring(0, 300);
                            usedDescs.add(r.description);
                            break;
                        }
                    }
                    if (!r.description) {
                        let node = link;
                        for (let depth = 0; depth < 3 && node?.parentElement; depth++) {
                            node = node.parentElement;
                            const desc = findBestDescription(node);
                            if (desc && desc !== r.title && desc.length > 20 && !usedDescs.has(desc)) {
                                r.description = desc.substring(0, 300);
                                usedDescs.add(r.description);
                                break;
                            }
                        }
                    }
                    if (r.description) break;
                }
            } catch(e) {}
        }

        // Sibling text extraction: check text nodes near the title link
        if (!r.description && r.url) {
            try {
                const links = [];
                document.querySelectorAll('a[href]').forEach(a => {
                    if (a.href === r.url) links.push(a);
                });
                for (const link of links) {
                    let node = link.parentElement;
                    for (let depth = 0; depth < 4 && node; depth++) {
                        const texts = [];
                        node.childNodes.forEach(child => {
                            if (child === link || child.contains?.(link)) return;
                            const t = (child.textContent || '').trim();
                            if (t.length >= 40 && t.length <= 300 && !DESC_REJECT.test(t) && t !== r.title) {
                                texts.push(t);
                            }
                        });
                        if (texts.length > 0) {
                            const best = texts.sort((a, b) => b.length - a.length)[0];
                            if (!usedDescs.has(best)) {
                                r.description = best.substring(0, 300);
                                usedDescs.add(r.description);
                                break;
                            }
                        }
                        node = node.parentElement;
                    }
                    if (r.description) break;
                }
            } catch(e) {}
        }

        // og:description fallback from Phase 4 fetch
        if (!r.description && r._ogDescription) {
            const d = r._ogDescription;
            if (d.length > 20 && d !== r.title && !usedDescs.has(d)) {
                r.description = d.substring(0, 300);
                usedDescs.add(r.description);
            }
        }

        if (!r.description && results.length <= 2) {
            const metaDesc = document.querySelector('meta[name="description"]')?.content
                || document.querySelector('meta[property="og:description"]')?.content || '';
            if (metaDesc.length > 20 && !usedDescs.has(metaDesc)) {
                r.description = metaDesc.substring(0, 300);
                usedDescs.add(r.description);
            }
        }
    });

    // Final cleanup pass on descriptions
    results.forEach(r => {
        if (r.description) {
            r.description = r.description
                .replace(/\s+/g, ' ')
                .replace(/\s*[A-Z][a-z]+\s+[A-Z][a-z]+\/(AP|AFP|Reuters|Getty|EPA|Bloomberg)\s*$/i, '')
                .trim()
                .substring(0, 300);
        }
    });

    // === Content quality scoring ===
    function scoreArticle(item) {
        let score = 0;
        if (item.title && item.title.length >= 30) score += 2;
        if (item.url) score += 2;
        if (item.image) score += 3;
        if (item.description) score += 2;
        if (item._locked) score += 1;
        if (/\/(CNN|AP|AFP|Reuters)\s*(Underscored|Wire)/i.test(item.title)) score -= 5;
        if (/^[A-Z][a-z]+\s[A-Z][a-z]+\//.test(item.title)) score -= 4;
        if (item.title && item.title.length < 20) score -= 2;
        if (!item.url || item.url === '') score -= 3;
        if (/\.(pdf|mp3|mp4)$/i.test(item.url || '')) score -= 3;
        if (/subscribe|sign.?up|newsletter|games|puzzles|crossword/i.test(item.title)) score -= 5;
        if (/shop|deal|sale|coupon|promo|discount|underscored/i.test(item.url || '')) score -= 4;
        return score;
    }

    results.forEach(r => { r._score = scoreArticle(r); });
    results.sort((a, b) => b._score - a._score);
    const qualityFiltered = results.filter(r => r._score >= 2);
    const scoredResults = qualityFiltered.length >= LIMIT ? qualityFiltered : results;

    const source = '__SOURCE__';
    const returned = scoredResults.slice(0, LIMIT).map(item => ({
        title: item.title,
        url: item.url,
        image: item.image,
        description: item.description || item._ogDescription || '',
        source: source || undefined,
        locked: item._locked || false,
        score: item._score
    }));
    const retImages = returned.map(r => r.image).filter(Boolean);
    const retUnique = new Set(retImages.map(u => imageKey(u)));
    const lockedCount = returned.filter(r => r.locked).length;
    const ogRecovered = scoredResults.slice(0, LIMIT).filter(r => r._ogRecovered).length;
    const filtered = results.length - qualityFiltered.length;
    return JSON.stringify({
        articles: returned,
        quality: {
            found: results.length,
            returned: returned.length,
            withImages: retImages.length,
            uniqueImages: retUnique.size,
            withDescriptions: returned.filter(r => r.description).length,
            lockedImages: lockedCount,
            ogRecovered,
            ogSkipped,
            filtered,
            budgetMs: BUDGET_MS,
            elapsed: Date.now() - (DEADLINE - BUDGET_MS)
        }
    });
})()
