(function() {
    const pairs = [];
    
    // Strategy 1: <dl> definition lists
    document.querySelectorAll('dl').forEach(dl => {
        const dts = dl.querySelectorAll('dt');
        dts.forEach(dt => {
            const dd = dt.nextElementSibling;
            if (dd && dd.tagName === 'DD') {
                const link = dd.querySelector('a[href]');
                pairs.push({
                    key: dt.textContent.trim(),
                    value: dd.textContent.trim(),
                    url: link ? link.href : null
                });
            }
        });
    });
    
    // Strategy 2: Label-value patterns (th/td pairs in 2-column tables)
    document.querySelectorAll('table').forEach(table => {
        const rows = table.querySelectorAll('tr');
        rows.forEach(tr => {
            const cells = tr.querySelectorAll('th, td');
            if (cells.length === 2) {
                const key = cells[0].textContent.trim();
                const val = cells[1].textContent.trim();
                if (key && val && key.length < 100) {
                    const link = cells[1].querySelector('a[href]');
                    pairs.push({ key, value: val, url: link ? link.href : null });
                }
            }
        });
    });
    
    // Strategy 3: Labeled spans/divs (class containing "label", "key", "name" + sibling "value")
    document.querySelectorAll('[class*="label"], [class*="key"], [class*="name"]').forEach(el => {
        const sibling = el.nextElementSibling;
        if (sibling) {
            const key = el.textContent.trim();
            const val = sibling.textContent.trim();
            if (key && val && key.length < 80 && val.length < 500) {
                pairs.push({ key, value: val, url: null });
            }
        }
    });
    
    // Deduplicate by key
    const seen = new Set();
    const unique = pairs.filter(p => {
        if (seen.has(p.key)) return false;
        seen.add(p.key);
        return true;
    });
    
    return JSON.stringify(unique.slice(0, 100));
})()
