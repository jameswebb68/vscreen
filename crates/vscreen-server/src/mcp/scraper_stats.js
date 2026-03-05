(function() {
    const stats = [];
    
    // Strategy 1: Elements with prominent numbers + labels
    const numPattern = /^[\$€£¥]?\s*[\d,.]+[%KMBTkmbt]?\s*$/;
    
    // Look for stat-card patterns (div with a large number + label)
    document.querySelectorAll('[class*="stat"], [class*="metric"], [class*="kpi"], [class*="summary"], [class*="highlight"], [class*="hero"]').forEach(el => {
        const texts = Array.from(el.querySelectorAll('*')).map(e => ({
            text: e.textContent.trim(),
            tag: e.tagName.toLowerCase(),
            fontSize: parseFloat(window.getComputedStyle(e).fontSize || '16')
        })).filter(t => t.text.length > 0 && t.text.length < 50);
        
        // Find the number (largest font or matches number pattern)
        const numEl = texts.find(t => numPattern.test(t.text)) || texts.find(t => t.fontSize > 24);
        const labelEl = texts.find(t => t !== numEl && !numPattern.test(t.text) && t.text.length < 60);
        
        if (numEl) {
            stats.push({
                label: labelEl ? labelEl.text : null,
                value: numEl.text,
                unit: null,
                trend: null
            });
        }
    });
    
    // Strategy 2: Standalone large numbers with adjacent labels
    document.querySelectorAll('h1, h2, h3, [class*="count"], [class*="number"], [class*="amount"], [class*="price"]').forEach(el => {
        const text = el.textContent.trim();
        if (numPattern.test(text) && text.length < 30) {
            const parent = el.parentElement;
            const siblings = parent ? Array.from(parent.children).filter(c => c !== el) : [];
            const label = siblings.find(s => !numPattern.test(s.textContent.trim()) && s.textContent.trim().length < 60);
            stats.push({
                label: label ? label.textContent.trim() : null,
                value: text,
                unit: null,
                trend: null
            });
        }
    });
    
    // Deduplicate
    const seen = new Set();
    const unique = stats.filter(s => {
        const key = s.value + (s.label || '');
        if (seen.has(key)) return false;
        seen.add(key);
        return true;
    });
    
    return JSON.stringify(unique.slice(0, 50));
})()
