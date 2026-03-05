import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import DataTable from '$lib/components/viz/DataTable.svelte';
import type { TableColumn, TableRow } from '$lib/types/index.js';

describe('DataTable', () => {
	const columns: TableColumn[] = [
		{ key: 'name', label: 'Name', sortable: true },
		{ key: 'age', label: 'Age', sortable: true },
		{ key: 'role', label: 'Role', sortable: false }
	];

	const rows: TableRow[] = [
		{ name: 'Alice', age: 30, role: 'Engineer' },
		{ name: 'Bob', age: 25, role: 'Designer' },
		{ name: 'Charlie', age: 35, role: 'Manager' }
	];

	it('renders title when provided', () => {
		render(DataTable, { columns, rows, title: 'Team Members' });
		expect(screen.getByText('Team Members')).toBeInTheDocument();
	});

	it('does not render title when omitted', () => {
		render(DataTable, { columns, rows });
		const heading = document.querySelector('h2');
		expect(heading).not.toBeInTheDocument();
	});

	it('renders all column headers', () => {
		render(DataTable, { columns, rows });
		expect(screen.getByText('Name')).toBeInTheDocument();
		expect(screen.getByText('Age')).toBeInTheDocument();
		expect(screen.getByText('Role')).toBeInTheDocument();
	});

	it('renders row data correctly', () => {
		render(DataTable, { columns, rows });
		expect(screen.getByText('Alice')).toBeInTheDocument();
		expect(screen.getByText('30')).toBeInTheDocument();
		expect(screen.getByText('Engineer')).toBeInTheDocument();
		expect(screen.getByText('Bob')).toBeInTheDocument();
		expect(screen.getByText('Charlie')).toBeInTheDocument();
	});

	it('supports sorting by clicking sortable column header', () => {
		render(DataTable, { columns, rows });
		const nameHeader = screen.getByText('Name');
		fireEvent.click(nameHeader);
		expect(screen.getByText('↑')).toBeInTheDocument();
	});

	it('sorts ascending then descending on repeated clicks', () => {
		render(DataTable, { columns, rows });
		const nameHeader = screen.getByText('Name');
		fireEvent.click(nameHeader);
		expect(screen.getByText('↑')).toBeInTheDocument();
		fireEvent.click(nameHeader);
		expect(screen.getByText('↓')).toBeInTheDocument();
	});

	it('does not sort non-sortable columns', () => {
		render(DataTable, { columns, rows });
		const roleHeader = screen.getByText('Role');
		fireEvent.click(roleHeader);
		expect(screen.queryByText('↑')).not.toBeInTheDocument();
		expect(screen.queryByText('↓')).not.toBeInTheDocument();
	});

	it('paginates when rows exceed pageSize', () => {
		const manyRows: TableRow[] = Array.from({ length: 25 }, (_, i) => ({
			name: `User ${i}`,
			age: 20 + i,
			role: 'Member'
		}));
		render(DataTable, { columns, rows: manyRows, pageSize: 10 });
		expect(screen.getByText('Prev')).toBeInTheDocument();
		expect(screen.getByText('Next')).toBeInTheDocument();
	});

	it('shows correct page count', () => {
		const manyRows: TableRow[] = Array.from({ length: 25 }, (_, i) => ({
			name: `User ${i}`,
			age: 20 + i,
			role: 'Member'
		}));
		render(DataTable, { columns, rows: manyRows, pageSize: 10 });
		expect(screen.getByText('1 / 3')).toBeInTheDocument();
	});

	it('handles empty rows gracefully', () => {
		const { container } = render(DataTable, { columns, rows: [] });
		const table = container.querySelector('table');
		expect(table).toBeInTheDocument();
		const tbody = container.querySelector('tbody');
		expect(tbody?.children).toHaveLength(0);
	});

	it('shows Prev and Next buttons for pagination', () => {
		const manyRows: TableRow[] = Array.from({ length: 30 }, (_, i) => ({
			name: `User ${i}`,
			age: 20 + i,
			role: 'Member'
		}));
		render(DataTable, { columns, rows: manyRows, pageSize: 10 });
		const prevBtn = screen.getByRole('button', { name: /prev/i });
		const nextBtn = screen.getByRole('button', { name: /next/i });
		expect(prevBtn).toBeInTheDocument();
		expect(nextBtn).toBeInTheDocument();
	});
});
