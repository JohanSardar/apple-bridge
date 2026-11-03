import json
import sys


def check(metadata):
    names = {p['id']: p['name'] for p in metadata['packages']}
    edges = {n['id']: n['dependencies'] for n in metadata['resolve']['nodes']}
    for root in metadata['workspace_members']:
        if not (names[root].startswith('lab-') or names[root] == 'bridge-lab'):
            continue
        pending = [(root, [names[root]])]
        visited = set()
        while pending:
            node, path = pending.pop()
            if node in visited:
                continue
            visited.add(node)
            if names[node].startswith('ab-'):
                raise ValueError('harness independence violated: ' + ' -> '.join(path))
            pending.extend((dep, path + [names[dep]]) for dep in edges[node])


if __name__ == '__main__':
    check(json.load(sys.stdin))
