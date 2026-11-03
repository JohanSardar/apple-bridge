import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location('independence', Path(__file__).with_name('check-independence.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def graph(product):
    return {'packages': [{'id': 'lab', 'name': 'lab-wire'}, {'id': 'shim', 'name': 'external-shim'}, {'id': 'product', 'name': product}], 'workspace_members': ['lab'], 'resolve': {'nodes': [{'id': 'lab', 'dependencies': ['shim']}, {'id': 'shim', 'dependencies': ['product']}, {'id': 'product', 'dependencies': ['shim']}]}}


module.check(graph('serde'))
try:
    module.check(graph('ab-wire'))
except ValueError as error:
    assert 'lab-wire -> external-shim -> ab-wire' in str(error)
else:
    raise AssertionError('transitive product dependency accepted')
print('independence positive, transitive negative and cycle probes passed')
