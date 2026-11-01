import json
import os
import unittest
from unittest.mock import patch
import activate


class ActivationTest(unittest.TestCase):
    def test_no_ui_service_is_used_without_explicit_configuration(self):
        with patch.dict(os.environ, {"AB_WDA_URL": ""}), patch.object(activate.urllib.request, "urlopen") as request:
            activate.activate("bridge")
        request.assert_not_called()

    def test_activation_requires_observed_foreground_state(self):
        def response(value):
            result = unittest.mock.MagicMock()
            result.__enter__.return_value.read.return_value = json.dumps({"value": value}).encode()
            return result
        for state, passed in [(4, True), (3, False)]:
            with patch.dict(os.environ, AB_WDA_URL="http://127.0.0.1:8101", AB_WDA_SESSION="test"), patch.object(activate.urllib.request, "urlopen", side_effect=[response(None), response(state)]) as request:
                if passed:
                    activate.activate("bridge")
                else:
                    with self.assertRaisesRegex(RuntimeError, "foreground"):
                        activate.activate("bridge")
                self.assertEqual(json.loads(request.call_args_list[0].args[0].data), {"bundleId": "bridge"})

    def test_ui_service_failure_does_not_qualify_activation(self):
        result = unittest.mock.MagicMock()
        result.__enter__.return_value.read.return_value = b'{"value":{"error":"invalid session id"}}'
        with patch.dict(os.environ, AB_WDA_URL="http://127.0.0.1:8101", AB_WDA_SESSION="stale"), patch.object(activate.urllib.request, "urlopen", return_value=result):
            with self.assertRaisesRegex(RuntimeError, "failed"):
                activate.activate("bridge")


if __name__ == "__main__":
    unittest.main()
