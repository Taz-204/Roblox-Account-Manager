import unittest

from roblox_lookup import parse_processes


class ParseProcessesTests(unittest.TestCase):
    def test_parses_helper_output(self):
        text = (
            '[{"pid":1234,"start":"2026-11-15T16:00:00.0000000+00:00",'
            '"window_title":"Roblox","command_line":"RobloxPlayerBeta.exe -b 42"}]'
        )
        procs = parse_processes(text)
        self.assertEqual(len(procs), 1)
        self.assertEqual(procs[0].pid, 1234)
        self.assertEqual(procs[0].window_title, "Roblox")
        self.assertEqual(procs[0].command_line, "RobloxPlayerBeta.exe -b 42")

    def test_empty_array_means_no_games(self):
        self.assertEqual(parse_processes("[]"), [])

    def test_null_fields_stay_none(self):
        procs = parse_processes('[{"pid":5,"start":"","window_title":null,"command_line":null}]')
        self.assertIsNone(procs[0].window_title)
        self.assertIsNone(procs[0].command_line)

    def test_escaped_quotes_in_command_line(self):
        text = '[{"pid":7,"start":"","window_title":"x","command_line":"a \\"b\\" c"}]'
        self.assertEqual(parse_processes(text)[0].command_line, 'a "b" c')


if __name__ == "__main__":
    unittest.main()
