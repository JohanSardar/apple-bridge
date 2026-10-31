import unittest

from check import direct_commands


class DirectIntentCoverageTest(unittest.TestCase):
    def test_deferred_approval_handlers_are_not_direct_intents(self):
        self.assertEqual(direct_commands('deferredHandlers: ["iphone.notify": deliver]'), set())

    def test_direct_handlers_remain_required(self):
        self.assertEqual(direct_commands('handlers: ["iphone.ping": ping]'), {"iphone.ping"})

    def test_command_prefix_is_not_a_registered_operation(self):
        self.assertEqual(direct_commands('handlers: ["iphone.ping": ping]; op.hasPrefix("iphone.capture.")'), {"iphone.ping"})


if __name__ == "__main__":
    unittest.main()
