package com.moscait.vej.demo.service;

import static org.junit.jupiter.api.Assertions.*;

import com.moscait.vej.demo.domain.CallCategory;
import com.moscait.vej.demo.domain.CallExample;
import java.util.EnumMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.junit.jupiter.api.Test;

class CustomerServiceExamplesTest {

    @Test
    void suppliesTwentyFiveDistinctExamplesBalancedAcrossAllFiveCategories() {
        List<CallExample> all = CustomerServiceExamples.all();
        assertTrue(all.size() >= 20, "Must provide at least 20 examples");
        assertEquals(25, all.size());

        Set<String> ids = new HashSet<>();
        Map<CallCategory, Integer> counts = new EnumMap<>(CallCategory.class);
        for (CallExample ex : all) {
            assertTrue(ids.add(ex.id()), "Duplicate ID: " + ex.id());
            assertNotNull(ex.expectedCategory());
            assertFalse(ex.transcript().isBlank());
            counts.merge(ex.expectedCategory(), 1, Integer::sum);
        }

        for (CallCategory category : CallCategory.values()) {
            assertEquals(5, counts.getOrDefault(category, 0), "Expected 5 examples for " + category);
        }
    }
}
