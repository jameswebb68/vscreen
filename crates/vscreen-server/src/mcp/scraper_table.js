(function() {
    const tables = document.querySelectorAll('table');
    const results = [];
    
    for (const table of Array.from(tables).slice(0, 10)) {
        const headers = Array.from(table.querySelectorAll('thead th, thead td, tr:first-child th'))
            .map(th => th.textContent.trim());
        
        // If no thead, try first row
        if (headers.length === 0) {
            const firstRow = table.querySelector('tr');
            if (firstRow) {
                Array.from(firstRow.querySelectorAll('th, td')).forEach(cell => {
                    headers.push(cell.textContent.trim());
                });
            }
        }
        
        const rows = [];
        const bodyRows = table.querySelectorAll('tbody tr, tr');
        const startIdx = headers.length > 0 ? (table.querySelector('thead') ? 0 : 1) : 0;
        
        Array.from(bodyRows).slice(startIdx, startIdx + 200).forEach(tr => {
            const cells = Array.from(tr.querySelectorAll('td, th')).map(td => {
                const link = td.querySelector('a[href]');
                const val = td.textContent.trim();
                return link ? { text: val, href: link.href } : val;
            });
            if (cells.length > 0 && cells.some(c => typeof c === 'string' ? c.length > 0 : c.text.length > 0)) {
                rows.push(cells);
            }
        });
        
        if (rows.length > 0) {
            results.push({
                columns: headers,
                rows: rows,
                row_count: rows.length,
                caption: table.querySelector('caption')?.textContent?.trim() || null
            });
        }
    }
    
    return JSON.stringify(results);
})()
