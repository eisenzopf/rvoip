"""score.caller_config must never exceed the caller's own media-pool check (the baseline1 void trials:
max_concurrent 60,080 > 60,000 slots at 9,680 sps refused every caller at start)."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import profiles  # noqa: E402
import score  # noqa: E402


def slots(config):
    ports = config['media_port_end'] - config['media_port_start'] + 1
    n = config['connections_per_source']
    return len(config['bind_ips']) * n * ((ports // n) & ~1)


class CallerConfigTests(unittest.TestCase):
    def test_max_concurrent_fits_the_media_pool_at_every_rate(self):
        for row_id in ('R1', 'R2', 'R5', 'R6'):
            for rate in range(400, 40001, 400):
                stages = [{'cps': rate / 4, 'seconds': 30}, {'cps': rate / 2, 'seconds': 180}]
                config = score.caller_config(profiles.ROWS[row_id], 'run', ['10.0.0.1', '10.0.0.2'], 8,
                                             '10.0.0.9', 5061, stages, '/out')
                self.assertLessEqual(config['max_concurrent'], slots(config), (row_id, rate))
                self.assertLessEqual(config['max_concurrent'], 35000 * 2, (row_id, rate))

    def test_media_rows_keep_room_for_the_hold(self):
        stages = [{'cps': 1500, 'seconds': 180}]
        config = score.caller_config(profiles.ROWS['R2'], 'run', ['10.0.0.1', '10.0.0.2'], 8, '10.0.0.9', 5061,
                                     stages, '/out')
        self.assertGreaterEqual(config['max_concurrent'], 1500 * 10)


if __name__ == '__main__':
    unittest.main()
