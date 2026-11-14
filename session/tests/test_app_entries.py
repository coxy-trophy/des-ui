"""Files and the calculator are listed as applications, and Files opens folders."""
import configparser
import pathlib
import unittest

SESSION = pathlib.Path(__file__).resolve().parent.parent


def entry(name):
    parser = configparser.ConfigParser(interpolation=None)
    parser.optionxform = str
    parser.read(SESSION / name)
    return parser['Desktop Entry']


class AppEntryTests(unittest.TestCase):
    def test_files_opens_folders_in_a_window_of_its_own(self):
        files = entry('des-files.desktop')
        self.assertEqual(files['Exec'], '/usr/local/bin/des-ui --files %U')
        self.assertIn('inode/directory;', files['MimeType'])
        # The dock knows the window by its app id.
        self.assertEqual(files['StartupWMClass'], 'des-files')
        self.assertIn('FileManager;', files['Categories'])

    def test_the_calculator_asks_the_running_desktop(self):
        calculator = entry('des-calculator.desktop')
        self.assertEqual(calculator['Exec'], '/usr/local/bin/des-ui-ctl calculator')
        self.assertEqual(calculator['NoDisplay'], 'false')
        self.assertIn('Calculator;', calculator['Categories'])


if __name__ == '__main__':
    unittest.main()
